#!/usr/bin/env python3
"""WORLD static-def parser  (bucket-(c): state/province NAMES, terrain, wargoals/
casus-belli, MIO catalog).

Closes the remaining bucket-(c) world-data + catalog items that aren't tech/
event/scripted-gui/idea/trait:

  state_names    : STATE_<id> loc -> {id: name}  (state/province panel + map)
  vp_names       : VICTORY_POINTS_<provid> loc -> {provid: name}
  terrain        : common/terrain `categories={ <t>={ movement_cost combat_width
                   units{} color } }` -> per-terrain combat modifiers (the panel/
                   map terrain rows)
  wargoals       : common/wargoals `wargoal_types={ <id>={...} }` -> casus-belli
                   catalog (generate_cost, threat, allowed/available presence)
  mio            : common/military_industrial_organization/organizations ->
                   MIO id -> include, icon, allowed/visible presence, traits ref
                   (the insignia/trait-bearing org catalog; gfx insignia paths are
                   resolved by the sprite layer, this is the def side).

stdlib only.
"""

from __future__ import annotations

import json
import re
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


def _ordered(layers, subdir, exts=(".txt",)):
    # path-boundary-safe load order (vanilla before mod), shared resolver helper.
    return R.ordered_files(layers, subdir, exts)


def _names_from_loc(loc, prefix):
    rx = re.compile(r"^" + re.escape(prefix) + r"(\d+)$")
    out = {}
    for k, v in loc.by_key.items():
        m = rx.match(k)
        if m:
            out[m.group(1)] = loc._COLOR.sub("", v)
    return out


def _terrain(layers):
    out = {}
    for rel, abspath in _ordered(layers, "common/terrain"):
        try:
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[world] WARN terrain {rel}: {e}", file=sys.stderr)
            continue
        cats = root.get("categories")
        if not isinstance(cats, Node):
            continue
        for tid, body in cats.entries:
            if not isinstance(body, Node):
                continue
            fixed = {"color", "sound_type", "type", "units", "is_water",
                     "perm_dust", "sprite", "map_color"}
            mods = {k: _plain(v) for k, v in body.entries
                    if k and k.lower() not in fixed}
            out[tid] = {
                "id": tid,
                "movement_cost": body.get("movement_cost"),
                "combat_width": body.get("combat_width"),
                "modifiers": mods,
                "source_file": rel,
            }
    return out


def _wargoals(layers):
    out = {}
    for rel, abspath in _ordered(layers, "common/wargoals"):
        try:
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[world] WARN wargoals {rel}: {e}", file=sys.stderr)
            continue
        wt = root.get("wargoal_types")
        if not isinstance(wt, Node):
            continue
        for wid, body in wt.entries:
            if not isinstance(body, Node):
                continue
            out[wid] = {
                "id": wid,
                "generate_cost": body.get("generate_cost"),
                "threat": body.get("threat"),
                "expire": body.get("expire"),
                "war_support_immediate": body.get("war_support_immediate"),
                "has_allowed": body.get("allowed") is not None,
                "has_available": body.get("available") is not None,
                "take_states": body.get("take_states") is not None,
                "source_file": rel,
            }
    return out


def _mio(layers):
    out = {}
    for subdir in ("common/military_industrial_organization/organizations",
                   "common/military_industrial_organization"):
        for rel, abspath in _ordered(layers, subdir):
            if Path(rel).parent.as_posix() != subdir:
                continue
            try:
                root = R.parse_file(abspath)
            except Exception:
                continue
            for oid, body in root.entries:
                if not isinstance(body, Node) or not oid:
                    continue
                out[oid] = {
                    "id": oid,
                    "include": _scalar(body.get("include")),
                    "icon": _scalar(body.get("icon")),
                    "name": _scalar(body.get("name")),
                    "has_allowed": body.get("allowed") is not None,
                    "has_visible": body.get("visible") is not None,
                    "has_traits": body.get("research_categories") is not None
                    or body.get("equipment_type") is not None,
                    "source_file": rel,
                }
    return out


def parse_world_defs(layers, loc) -> dict:
    state_names = _names_from_loc(loc, "STATE_")
    vp_names = _names_from_loc(loc, "VICTORY_POINTS_")
    terrain = _terrain(layers)
    wargoals = _wargoals(layers)
    mio = _mio(layers)
    return {
        "meta": {
            "state_names": len(state_names),
            "vp_names": len(vp_names),
            "terrain": len(terrain),
            "wargoals": len(wargoals),
            "mio": len(mio),
        },
        "state_names": dict(sorted(state_names.items(), key=lambda kv: int(kv[0]))),
        "vp_names": dict(sorted(vp_names.items(), key=lambda kv: int(kv[0]))),
        "terrain": dict(sorted(terrain.items())),
        "wargoals": dict(sorted(wargoals.items())),
        "mio": dict(sorted(mio.items())),
    }


def build(vanilla=None, mods=None):
    layers = R.build_layers(vanilla=vanilla, mods=mods)
    loc = R.load_loc(layers)
    return parse_world_defs(layers, loc)


if __name__ == "__main__":
    out = build()
    outp = Path(__file__).resolve().parent / "out" / "world_defs.json"
    outp.parent.mkdir(exist_ok=True)
    outp.write_text(json.dumps(out, ensure_ascii=False, indent=1), encoding="utf-8")
    m = out["meta"]
    print(f"world_defs: {m['state_names']} states, {m['vp_names']} VPs, "
          f"{m['terrain']} terrain, {m['wargoals']} wargoals, {m['mio']} MIO "
          f"-> {outp.name}")
