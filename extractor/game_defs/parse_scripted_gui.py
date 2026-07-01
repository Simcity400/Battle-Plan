#!/usr/bin/env python3
"""SCRIPTED-GUI element->variable map for scripted GUI and mod overlays.

For each ``scripted_gui = { <gui_name> = {...} }`` definition in the effective
mod-over-vanilla ``common/scripted_guis/*.txt`` set, we extract the binding key
that lets the app light up an overlay: which variable feeds which element, and
which effect/trigger reads/writes what.

Per gui_name:
  context_type          : player_context | decision_category | state_context ...
  window_name           : the interface .gui window this binds to
  parent_window_token   : where it docks (if any)
  dirty                 : the var that flags the gui dirty (if present)
  visible_vars / enabled_vars : variables read in visible={}/enabled={} (the
                          show/hide condition — what the app must evaluate)
  reads / writes        : ALL variables read/written anywhere in the def
                          (full op set: check/set/add/sub/mul/div/clear/temp...)
  effects               : { effect_name: {reads, writes} } — effect_name of form
                          ``<element>_click`` fires on clicking ``<element>``
  triggers              : { trigger_name: {reads} }
  elements              : { element_name: {image, frame, text, ...} } from
                          properties={} — incl. dynamic ``[GET_X]`` image tokens
  dynamic_loc_calls     : the ``[GET_X]`` scripted-localisation function names
                          referenced (resolved against common/scripted_localisation
                          so the app knows these are scripted-loc fns, NOT raw vars)

source_file recorded per gui so mod-over-vanilla is auditable.

stdlib only (+ resolver, clausewitz_ops, paradox_parser).
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

import resolver as R
from paradox_parser import Node
from clausewitz_ops import harvest_variables, operator_triples

_GET = re.compile(r"\[([A-Za-z_][A-Za-z0-9_]*)\]")    # [GET_X] dynamic loc token
# [?scope.var?] / [?var?] inline variable reference in a property image/text.
_QVAR = re.compile(r"\[\?\s*([A-Za-z_][A-Za-z0-9_.]*)")


def _scalar(v):
    return v if isinstance(v, str) else None


def _vars_in(node: Node):
    reads, writes = set(), set()
    harvest_variables(node, reads, writes)
    return sorted(reads), sorted(writes)


def _visible_enabled_vars(node: Node, field: str):
    """Vars/flags read in visible={}/enabled={} (harvest_variables now captures
    flags recursively, incl. nested `ROOT={ has_country_flag=X }`)."""
    blk = node.get(field)
    if not isinstance(blk, Node):
        return []
    r, w = set(), set()
    harvest_variables(blk, r, w)
    return sorted(r | w)


def _parse_properties(blk: Node, dynamic_calls: set, prop_var_reads: set):
    """properties={ <element>={ image= frame= text= } } -> element map. Captures
    [GET_X] scripted-loc tokens AND [?scope.var?] inline variable reads (Codex
    #5) so a property image bound to a variable surfaces that var as a read."""
    elements = {}
    if not isinstance(blk, Node):
        return elements
    for elem, ebody in blk.entries:
        if not isinstance(ebody, Node):
            continue
        rec = {}
        for fk, fv in ebody.entries:
            if isinstance(fv, str):
                rec[fk.lower()] = fv
                for m in _GET.finditer(fv):
                    dynamic_calls.add(m.group(1))
                for m in _QVAR.finditer(fv):
                    prop_var_reads.add(m.group(1))
        if rec:
            elements[elem] = rec
    return elements


def parse_gui_def(name: str, node: Node, source_file: str,
                  scripted_loc_names: set) -> dict:
    reads, writes = _vars_in(node)
    rec = {
        "name": name,
        "context_type": _scalar(node.get("context_type")),
        "window_name": _scalar(node.get("window_name")),
        "parent_window_token": _scalar(node.get("parent_window_token")),
        "dirty": _scalar(node.get("dirty")),
        "visible_vars": _visible_enabled_vars(node, "visible"),
        "enabled_vars": _visible_enabled_vars(node, "enabled"),
        "reads": reads,
        "writes": writes,
        "effects": {},
        "triggers": {},
        "elements": {},
        "dynamic_loc_calls": [],
        "source_file": source_file,
    }
    dyn = set()
    # effects = { <effname> = {...} }
    eff = node.get("effects")
    if isinstance(eff, Node):
        for ename, ebody in eff.entries:
            if isinstance(ebody, Node):
                r, w = _vars_in(ebody)
                # engine convention: <element>_click / <element>_right_click fire
                # on (right-)clicking <element>; recover the element + which click.
                elem, click = None, None
                if ename.endswith("_right_click"):
                    elem, click = ename[:-12], "right"
                elif ename.endswith("_click"):
                    elem, click = ename[:-6], "left"
                rec["effects"][ename] = {
                    "reads": r, "writes": w,
                    "on_click_of": elem, "click": click,
                }
    # triggers = { <trgname> = {...} } — names like <element>_visible /
    # <element>_enabled / <element>_click_enabled gate a specific element.
    trg = node.get("triggers")
    if isinstance(trg, Node):
        for tname, tbody in trg.entries:
            if isinstance(tbody, Node):
                r, _ = _vars_in(tbody)
                gate, elem = None, None
                for suf in ("_click_enabled", "_right_click_enabled",
                            "_visible", "_enabled"):
                    if tname.endswith(suf):
                        gate, elem = suf[1:], tname[:-len(suf)]
                        break
                rec["triggers"][tname] = {"reads": r, "gates_element": elem,
                                          "gate": gate}
    # properties (element map) + inline [?var?] reads
    prop_reads = set()
    rec["elements"] = _parse_properties(node.get("properties"), dyn, prop_reads)
    if prop_reads:
        rec["reads"] = sorted(set(rec["reads"]) | prop_reads)
    # classify dynamic-loc calls: which resolve to a scripted_localisation fn
    rec["dynamic_loc_calls"] = sorted(dyn)
    rec["dynamic_loc_calls_resolved"] = sorted(d for d in dyn if d in scripted_loc_names)
    return rec


def _load_scripted_loc_names(layers) -> set:
    """Names of defined_text entries in common/scripted_localisation so we can
    tell a `[GET_X]` apart from a raw variable (Codex C6)."""
    names = set()
    fm = R.effective_files(layers, "common/scripted_localisation", (".txt",))
    for rel, abspath in fm.items():
        try:
            root = R.parse_file(abspath)
        except Exception:
            continue
        for k, v in root.entries:
            if k.lower() == "defined_text" and isinstance(v, Node):
                nm = _scalar(v.get("name"))
                if nm:
                    names.add(nm)
    return names


def parse_scripted_guis(layers) -> dict:
    scripted_loc_names = _load_scripted_loc_names(layers)
    ordered = R.ordered_files(layers, "common/scripted_guis", (".txt",))
    guis: dict[str, dict] = {}
    collisions = []
    for rel, abspath in ordered:
        try:
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[scripted_gui] WARN {rel}: {e}", file=sys.stderr)
            continue
        # top level: scripted_gui = { <name> = {...}, ... }
        for k, v in root.entries:
            if k.lower() != "scripted_gui" or not isinstance(v, Node):
                continue
            for gname, gbody in v.entries:
                if not isinstance(gbody, Node):
                    continue
                rec = parse_gui_def(gname, gbody, rel, scripted_loc_names)
                if gname in guis and guis[gname]["source_file"] != rel:
                    collisions.append({"name": gname,
                                       "first": guis[gname]["source_file"],
                                       "override": rel})
                guis[gname] = rec
    return {
        "meta": {
            "gui_count": len(guis),
            "files_parsed": len(ordered),
            "scripted_loc_fns": len(scripted_loc_names),
            "name_collisions_last_wins": collisions,
        },
        "scripted_guis": dict(sorted(guis.items())),
    }


def build(vanilla=None, mods=None):
    layers = R.build_layers(vanilla=vanilla, mods=mods)
    return parse_scripted_guis(layers)


if __name__ == "__main__":
    out = build()
    outp = Path(__file__).resolve().parent / "out" / "scripted_guis.json"
    outp.parent.mkdir(exist_ok=True)
    outp.write_text(json.dumps(out, ensure_ascii=False, indent=1), encoding="utf-8")
    m = out["meta"]
    print(f"scripted_guis: {m['gui_count']} defs from {m['files_parsed']} files "
          f"-> {outp.name}")
