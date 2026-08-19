#!/usr/bin/env python3
"""the game save-file DIVISION oracle.

Extends the save oracle (see ``parse_save.py``) to extract the **fielded land
divisions** from a melted the game save, joined to their template name
and equipment archetypes. This is the authoritative ground-truth ("Architecture
A" oracle) for the order-of-battle layer that the separate live memory reader
will be validated against.

What it produces (under ``output/``):
  - ``divisions.json``   : one record per active division
  - aggregates embedded in ``divisions.json`` under ``"by_country"`` /
    ``"by_template"`` (counts + total strength), plus a top-level ``"meta"``.

Save structure (verified by inspecting a real melt):
  - Active divisions live at ``countries.<TAG>.units.division[]``. Each country
    block has exactly one ``units={ ... }`` whose *direct* children
    ``division={ ... }`` are the real fielded divisions. A real division has
    ``logical_country`` (the owner), ``location`` (province id),
    ``division_template_id={id,type}``, ``organisation``, ``strength`` and an
    ``equipment={ equipment={ id={id,type} amount=N } ... }`` list.
  - DECOY: ``division={ id=.. type=.. }`` handle-refs also appear inside
    ``experience_status.xp_by_template={ { combat=.. division={id,type} } }``.
    Those carry ONLY ``id``/``type`` and are excluded two ways: we only descend
    into the direct ``units={}`` block (never recurse into other country
    sub-blocks), AND we require ``logical_country`` to be present.
  - Templates: top-level ``division_templates={ division_template={ id={id,type}
    name="..." country="GER" role="garrison"(optional) ... } ... }``.
  - Equipment archetypes: top-level ``equipments={ <archetype_name>={
    id={id type=70} ... } ... }`` — the archetype NAME is the block KEY.

Joins use the full Clausewitz object key ``(id, type)`` (not bare id), per the
engine's reference semantics. ``div_id`` in the output is the bare id (unique in
practice within a save) but ``div_type`` is kept alongside it.

stdlib only. Reuses ``parse_save.py``'s tokenizer; adds nothing to it.

Usage:
    python parse_units.py [save_or_melted_path]
    python parse_units.py --rakaly /path/to/rakaly.exe --out-dir <dir> [save]
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import parse_save as ps
from parse_save import (
    Tokenizer,
    LBRACE,
    RBRACE,
    atom_value,
    iter_block_entries,
    _skip_block_body,
)



def _read_id_block(tz: Tokenizer):
    """Read an ``{ id=<n> type=<n> }`` block (LBRACE already consumed).

    Returns ``(id_int, type_int)`` with either possibly None. Tolerant of extra
    keys and of the block being malformed.
    """
    _id = None
    _type = None
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            _skip_block_body(tz)
            continue
        val = atom_value(opener)
        if val is None:
            continue
        if key == "id":
            _id = _to_int(val)
        elif key == "type":
            _type = _to_int(val)
    return _id, _type


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



def _parse_division_templates(tz: Tokenizer, templates: dict) -> None:
    """Parse ``division_templates={ division_template={...} ... }``.

    Fills ``templates[(id,type)] = {"name","country","role"}``.
    """
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key != "division_template":
            _skip_block_body(tz)
            continue
        name = country = role = loc_key = None
        tid = ttype = None
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                if k == "id":
                    tid, ttype = _read_id_block(tz)
                else:
                    _skip_block_body(tz)
                continue
            val = atom_value(op)
            if val is None:
                continue
            if k == "name":
                name = val
            elif k == "localization_key":
                loc_key = val
            elif k == "country":
                country = val
            elif k == "role":
                role = val
        if tid is not None:
            templates[(tid, ttype)] = {
                "name": name if name is not None else loc_key,
                "country": country, "role": role,
            }


def _parse_equipments(tz: Tokenizer, equip_index: dict) -> None:
    """Parse top-level ``equipments={ <archetype_name>={ id={id type} ... } }``.

    Fills ``equip_index[(id,type)] = archetype_name``.
    """
    for arch_name, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        eid = etype = None
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                if k == "id":
                    eid, etype = _read_id_block(tz)
                else:
                    _skip_block_body(tz)
                continue
        if eid is not None:
            equip_index[(eid, etype)] = arch_name


def _parse_countries_units(tz: Tokenizer, raw_divisions: list) -> None:
    """Walk ``countries={ <TAG>={ ... units={ division={..} ... } ... } }``.

    Appends each real division (as a raw dict, pre-join) to ``raw_divisions``.
    Only the *direct* ``units={}`` block of each country is descended into, so
    ``xp_by_template`` handle-ref decoys (elsewhere in the country block) are
    never seen.
    """
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                if k == "units":
                    _parse_units_block(tz, raw_divisions)
                else:
                    _skip_block_body(tz)


def _parse_units_block(tz: Tokenizer, raw_divisions: list) -> None:
    """Parse a single ``units={ division={..} division={..} ... }`` block."""
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "division":
                d = _parse_division_block(tz)
                if d is not None and d.get("owner"):
                    raw_divisions.append(d)
            else:
                _skip_block_body(tz)


def _parse_division_block(tz: Tokenizer):
    """Parse one ``division={ ... }`` block (LBRACE already consumed)."""
    div = {
        "div_id": None,
        "div_type": None,
        "owner": None,
        "location_province": None,
        "template_id": None,
        "organisation": None,
        "strength": None,
        "equipment": [],
    }
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "id":
                _id, _type = _read_id_block(tz)
                if div["div_id"] is None:
                    div["div_id"] = _id
                    div["div_type"] = _type
            elif k == "division_template_id":
                div["template_id"] = _read_id_block(tz)
            elif k == "equipment":
                div["equipment"] = _parse_equipment_list(tz)
            else:
                _skip_block_body(tz)
            continue
        val = atom_value(op)
        if val is None:
            continue
        if k == "logical_country":
            div["owner"] = val
        elif k == "location":
            div["location_province"] = _to_int(val)
        elif k == "organisation":
            div["organisation"] = _to_float(val)
        elif k == "strength":
            div["strength"] = _to_float(val)
    return div


def _parse_equipment_list(tz: Tokenizer):
    """Parse ``equipment={ equipment={ id={id,type} amount=N } ... }``."""
    out = []
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k == "equipment":
                eid = etype = None
                amount = None
                for kk, oo in iter_block_entries(tz):
                    if oo is LBRACE:
                        if kk == "id":
                            eid, etype = _read_id_block(tz)
                        else:
                            _skip_block_body(tz)
                        continue
                    v = atom_value(oo)
                    if v is None:
                        continue
                    if kk == "amount":
                        amount = _to_float(v)
                if eid is not None:
                    out.append((eid, etype, amount))
            else:
                _skip_block_body(tz)
    return out



def parse_units_text(text: str) -> dict:
    """Parse a melted/plaintext save string into the units result dict.

    Robust to section ordering: collects raw divisions, the template index, and
    the equipment index in a single top-level pass, then joins afterward.
    """
    tz = Tokenizer(text)
    templates: dict = {}
    equip_index: dict = {}
    raw_divisions: list = []
    meta = {"date": None, "version": None, "player": None,
            "player_countries": []}

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
        nxt = tz.peek()
        if nxt is not ps.EQUALS:
            continue
        tz.next()
        val = tz.next()
        if val is LBRACE:
            if key == "division_templates":
                _parse_division_templates(tz, templates)
            elif key == "equipments":
                _parse_equipments(tz, equip_index)
            elif key == "countries":
                _parse_countries_units(tz, raw_divisions)
            elif key == "player_countries":
                meta["player_countries"] = ps._read_atom_list(tz)
            else:
                _skip_block_body(tz)
        else:
            sval = atom_value(val)
            if sval is not None:
                if key == "date":
                    meta["date"] = sval
                elif key == "version":
                    meta["version"] = sval
                elif key == "player":
                    meta["player"] = sval

    unknown_template_ids = set()
    unknown_equipment_ids = set()
    divisions = []
    for d in raw_divisions:
        tkey = d["template_id"]
        tinfo = templates.get(tkey) if tkey else None
        if tkey and tinfo is None:
            unknown_template_ids.add(tkey)
        equipment = []
        for (eid, etype, amount) in d["equipment"]:
            arch = equip_index.get((eid, etype))
            if arch is None:
                unknown_equipment_ids.add((eid, etype))
            equipment.append({
                "archetype": arch,
                "equip_id": eid,
                "equip_type": etype,
                "amount": amount,
            })
        divisions.append({
            "div_id": d["div_id"],
            "div_type": d["div_type"],
            "owner": d["owner"],
            "location_province": d["location_province"],
            "state_id": None,
            "template_id": tkey[0] if tkey else None,
            "template_type": tkey[1] if tkey else None,
            "template_name": tinfo["name"] if tinfo else None,
            "template_role": tinfo["role"] if tinfo else None,
            "organisation": d["organisation"],
            "strength": d["strength"],
            "equipment": equipment,
        })

    return {
        "divisions": divisions,
        "templates": templates,
        "equip_index": equip_index,
        "meta": meta,
        "unknown_template_ids": sorted(unknown_template_ids),
        "unknown_equipment_ids": sorted(unknown_equipment_ids),
    }



def _load_province_to_state(script_dir: Path):
    """Load map_pipeline/assets/province_to_state.json -> {int prov: int state}.

    Returns (mapping, path_or_None). The map is read-only and optional; if it is
    missing, division state_id stays None and we say so in the report.
    """
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


def build_division_outputs(parsed: dict, prov_to_state: dict,
                           source: Path, was_melted: bool) -> dict:
    divisions = parsed["divisions"]

    for d in divisions:
        pid = d["location_province"]
        if pid is not None:
            d["state_id"] = prov_to_state.get(pid)

    by_country: dict = {}
    by_template: dict = {}
    for d in divisions:
        owner = d["owner"] or "??"
        c = by_country.setdefault(owner, {"count": 0, "total_strength": 0.0})
        c["count"] += 1
        c["total_strength"] += d["strength"] or 0.0

        tid = d["template_id"]
        ttype = d["template_type"]
        tk = f"{tid}:{ttype}" if tid is not None else "??"
        tinfo = parsed["templates"].get((tid, ttype))
        t = by_template.setdefault(tk, {
            "count": 0, "total_strength": 0.0,
            "name": d["template_name"], "role": d["template_role"],
            "country": tinfo["country"] if tinfo else None,
        })
        t["count"] += 1
        t["total_strength"] += d["strength"] or 0.0

    for c in by_country.values():
        c["total_strength"] = round(c["total_strength"], 2)
    for t in by_template.values():
        t["total_strength"] = round(t["total_strength"], 2)

    unknown_template_set = {tuple(t) for t in parsed["unknown_template_ids"]}
    templates_resolved = sum(
        1 for d in divisions
        if d["template_id"] is not None
        and (d["template_id"], d["template_type"]) not in unknown_template_set)
    templates_named = sum(
        1 for d in divisions if d["template_name"] is not None)
    n = len(divisions)
    meta = {
        "source_filename": source.name,
        "source_path_redacted": ps._redact(source),
        "was_melted": was_melted,
        "save_date": parsed["meta"].get("date"),
        "save_version": parsed["meta"].get("version"),
        "player": parsed["meta"].get("player"),
        "stats": {
            "divisions": n,
            "distinct_owners": len({d["owner"] for d in divisions}),
            "templates_in_save": len(parsed["templates"]),
            "equipment_archetypes": len(parsed["equip_index"]),
            "divisions_template_resolved": templates_resolved,
            "divisions_with_template_name": templates_named,
            "template_resolve_pct": round(100.0 * templates_resolved / n, 1)
            if n else 0.0,
            "divisions_with_state": sum(
                1 for d in divisions if d["state_id"] is not None),
            "unknown_template_ids": len(parsed["unknown_template_ids"]),
            "unknown_equipment_ids": len(parsed["unknown_equipment_ids"]),
        },
    }

    return {
        "meta": meta,
        "by_country": dict(sorted(by_country.items())),
        "by_template": by_template,
        "divisions": divisions,
    }


def validation_report(out: dict) -> str:
    st = out["meta"]["stats"]
    lines = []
    lines.append("=== DIVISION ORACLE VALIDATION ===")
    lines.append(f"source        : {out['meta']['source_filename']} "
                 f"(melted={out['meta']['was_melted']})")
    lines.append(f"save date     : {out['meta']['save_date']}  "
                 f"version={out['meta']['save_version']}")
    lines.append(f"divisions     : {st['divisions']}  (expect ~1654)")
    lines.append(f"distinct owners: {st['distinct_owners']}")
    lines.append(f"templates     : {st['templates_in_save']} in save, "
                 f"{st['divisions_template_resolved']} divisions' template_id "
                 f"resolved ({st['template_resolve_pct']}%), "
                 f"{st['divisions_with_template_name']} have a name")
    lines.append(f"equip archetypes: {st['equipment_archetypes']}  "
                 f"unknown equip ids: {st['unknown_equipment_ids']}  "
                 f"unknown template ids: {st['unknown_template_ids']}")
    lines.append(f"div w/ state   : {st['divisions_with_state']} "
                 f"(province->state joined)")

    bc = sorted(out["by_country"].items(),
                key=lambda kv: kv[1]["count"], reverse=True)
    lines.append("top owners (count / total strength):")
    for tag, agg in bc[:8]:
        lines.append(f"   {tag}: {agg['count']:>4} divs, "
                     f"str={agg['total_strength']}")

    lines.append("sample divisions:")
    for d in out["divisions"][:6]:
        eq0 = d["equipment"][0]["archetype"] if d["equipment"] else None
        lines.append(
            f"   #{d['div_id']} owner={d['owner']} prov={d['location_province']}"
            f" state={d['state_id']} tmpl={d['template_name']!r}"
            f" role={d['template_role']} org={d['organisation']}"
            f" str={d['strength']} eq0={eq0}")
    return "\n".join(lines)



def main(argv=None) -> int:
    ap = argparse.ArgumentParser(
        description="the game save -> divisions.json (order of battle oracle).")
    ap.add_argument("save", nargs="?", default=None,
                    help="Path to a .hoi4 save or melted plaintext. "
                         "If omitted, the most recent candidate is used.")
    ap.add_argument("--rakaly", default=None,
                    help="Path to rakaly executable (for binary saves).")
    ap.add_argument("--out-dir", default=None,
                    help="Output directory (default: <script>/output).")
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
            print("ERROR: no candidate save found in known locations.",
                  file=sys.stderr)
            return 2

    print(f"[units] resolved save : {ps._scrub(str(save_path))}")
    rakaly = ps.find_rakaly(args.rakaly)
    print(f"[units] rakaly        : "
          f"{ps._scrub(str(rakaly)) if rakaly else 'NOT FOUND'}")

    try:
        text, was_melted = ps.melt_if_needed(save_path, rakaly, out_dir)
    except RuntimeError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 3

    print(f"[units] melted        : {was_melted}  "
          f"({len(text):,} chars to parse)")
    parsed = parse_units_text(text)

    prov_to_state, ptos_path = _load_province_to_state(script_dir)
    if ptos_path is None:
        print("[units] note: province_to_state.json not found; "
              "division state_id will be null.")
    else:
        print(f"[units] prov->state   : {ps._scrub(str(ptos_path))} "
              f"({len(prov_to_state)} provinces)")

    out = build_division_outputs(parsed, prov_to_state, save_path, was_melted)

    (out_dir / "divisions.json").write_text(
        json.dumps(out, indent=1), encoding="utf-8")

    report = validation_report(out)
    print()
    print(report)
    print()
    print(f"[units] wrote: {ps._scrub(str(out_dir / 'divisions.json'))}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
