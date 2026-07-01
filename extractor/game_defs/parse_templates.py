#!/usr/bin/env python3
"""DIVISION / GARRISON TEMPLATE catalog  (bucket-(c): occupation garrison_log,
state-panel garrison, deployment template chrome).

Division templates (the unit a country garrisons / fields with) are declared in
``history/units/*.txt`` as ``division_template = { name= regiments={...}
support={...} }``. The garrison TEMPLATE the occupation system uses is a normal
division template selected per country; we catalog all templates with their
battalion composition + a battalion COUNT (the cost driver — exact IC needs the
sub_unit DB join, which we surface as the regiment list so the app can cost it).

Per template:
  name, owner_tag (from filename / context where derivable), division_names_group,
  regiments : { sub_unit: count } (front-line battalions), support : { sub_unit: count },
  battalion_count, support_count, width_slots (max x+1), source_file.

We also load ``common/units/*.txt`` sub_unit definitions (battalion->combat_width,
manpower, equipment need) so the app can compute the garrison cost (Codex B1
join). Sub-units kept as a side table.

stdlib only.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

import resolver as R
from paradox_parser import Node

_TAG_RE = re.compile(r"^([A-Z]{3})[_\.]")


def _scalar(v):
    return v if isinstance(v, str) else None


def _regiments(blk: Node):
    """regiments / support = { infantry = { x= y= } ... } -> {sub_unit: count}."""
    counts: dict[str, int] = {}
    max_col = -1
    if isinstance(blk, Node):
        for k, v in blk.entries:
            if isinstance(v, Node) and k:
                counts[k] = counts.get(k, 0) + 1
                x = v.get("x")
                if isinstance(x, str) and x.isdigit():
                    max_col = max(max_col, int(x))
    return counts, (max_col + 1 if max_col >= 0 else 0)


def _load_subunits(layers) -> dict:
    """sub_unit (battalion) defs for the garrison/template COST join."""
    subunits = {}
    for rel, abspath in R.ordered_files(layers, "common/units", (".txt",)):
        if Path(rel).parent.as_posix() != "common/units":  # top-level only
            continue
        try:
            root = R.parse_file(abspath)
        except Exception:
            continue
        for blk in root.get_all("sub_units"):
            if not isinstance(blk, Node):
                continue
            for su, body in blk.entries:
                if not isinstance(body, Node):
                    continue
                need = body.get("need")
                subunits[su] = {
                    "combat_width": body.get("combat_width"),
                    "manpower": body.get("manpower"),
                    "max_strength": body.get("max_strength"),
                    "need": {k: v for k, v in (need.entries
                             if isinstance(need, Node) else [])
                             if isinstance(v, str)},
                }
    return subunits


def _to_num(v):
    try:
        return float(v)
    except (TypeError, ValueError):
        return 0.0


def _cost(reg_counts, sup_counts, subunits):
    """Aggregate manpower + equipment NEED across the template's battalions via
    the subunit join (Codex #4: surface a derived cost, not just inputs)."""
    manpower = 0.0
    need: dict[str, float] = {}
    for counts in (reg_counts, sup_counts):
        for su, n in counts.items():
            su_def = subunits.get(su)
            if not su_def:
                continue
            manpower += _to_num(su_def.get("manpower")) * n
            for eq, qty in su_def["need"].items():
                need[eq] = need.get(eq, 0.0) + _to_num(qty) * n
    return manpower, {k: round(v, 2) for k, v in need.items()}


def parse_templates(layers) -> dict:
    templates = []
    by_name: dict[str, list] = {}
    subunits = _load_subunits(layers)

    for rel, abspath in R.ordered_files(layers, "history/units", (".txt",)):
        try:
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[templates] WARN {rel}: {e}", file=sys.stderr)
            continue
        stem = Path(rel).stem
        m = _TAG_RE.match(stem)
        owner = m.group(1) if m else None
        for k, v in root.entries:
            if k.lower() != "division_template" or not isinstance(v, Node):
                continue
            name = _scalar(v.get("name"))
            reg_counts, width = _regiments(v.get("regiments"))
            sup_counts, _ = _regiments(v.get("support"))
            manpower, eq_need = _cost(reg_counts, sup_counts, subunits)
            rec = {
                "name": name, "owner_tag": owner,
                "division_names_group": _scalar(v.get("division_names_group")),
                "regiments": reg_counts,
                "support": sup_counts,
                "battalion_count": sum(reg_counts.values()),
                "support_count": sum(sup_counts.values()),
                "width_slots": width,
                "manpower": int(manpower),
                "equipment_need": eq_need,   # derived cost (battalion join)
                "source_file": rel,
            }
            templates.append(rec)
            if name:
                # NOT lossy: one name (e.g. "Infantry Division") recurs across
                # owners/compositions — keep ALL under the name.
                by_name.setdefault(name, []).append(rec)

    return {
        "meta": {
            "template_count": len(templates),
            "unique_named": len(by_name),
            "subunit_defs": len(subunits),
            "files_parsed": len({t["source_file"] for t in templates}),
        },
        "templates": templates,
        "templates_by_name": by_name,
        "subunits": dict(sorted(subunits.items())),
    }


def build(vanilla=None, mods=None):
    layers = R.build_layers(vanilla=vanilla, mods=mods)
    return parse_templates(layers)


if __name__ == "__main__":
    out = build()
    outp = Path(__file__).resolve().parent / "out" / "templates.json"
    outp.parent.mkdir(exist_ok=True)
    outp.write_text(json.dumps(out, ensure_ascii=False, indent=1), encoding="utf-8")
    m = out["meta"]
    print(f"templates: {m['template_count']} ({m['unique_named']} unique names), "
          f"{m['subunit_defs']} subunit defs from {m['files_parsed']} files "
          f"-> {outp.name}")
