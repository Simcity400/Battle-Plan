#!/usr/bin/env python3
"""NATIONAL-FOCUS static-def parser (bucket-(c): national_focus view geometry +
PP cost / availability).

Distinct from the SAVE-side focus oracle (``save_oracle/parse_focus.py``, which
reports per-country COMPLETED/current focuses). THIS parser reads the static
DEFINITION from ``common/national_focus/*.txt``: each focus tree's nodes with
their xy GEOMETRY, prerequisites, mutual exclusion, PP ``cost``, the loc keys
(id + id_desc), icon, and reward presence — the node graph the focus view draws.

Per focus tree:
  id, country (where set), default, continuous_focus_position, shared_focus refs
Per focus:
  id, icon, overlay, text_icon, x, y, relative_position_id, authored base cost,
  prerequisites [[...] or...], mutually_exclusive, available/bypass presence,
  exact background style and four state sprites, search_filters,
  loc {title,desc}, source_file. The authored cost is not promoted to an
  effective runtime duration because decisions and scripted state can reduce it.

stdlib only.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import resolver as R
from paradox_parser import Node, coerce_scalar


def _scalar(v):
    return v if isinstance(v, str) else None


def _num(v):
    return coerce_scalar(v) if isinstance(v, str) else None


def _prereq_groups(node: Node):
    """`prerequisite = { focus = a focus = b }` is an OR group; multiple
    prerequisite blocks are AND-ed. Return list of OR-groups (list of ids)."""
    groups = []
    for p in node.get_all("prerequisite"):
        if isinstance(p, Node):
            groups.append([v for k, v in p.entries
                           if k.lower() == "focus" and isinstance(v, str)])
    return groups


def parse_focus(node: Node, loc, rel) -> dict:
    fid = _scalar(node.get("id"))
    if not fid:
        return None
    title = loc.resolve(fid) if loc else None
    desc = loc.resolve(fid + "_desc") if loc else None
    return {
        "id": fid,
        "icon": _scalar(node.get("icon")),
        "overlay": _scalar(node.get("overlay")),
        "text_icon": _scalar(node.get("text_icon")),
        "x": _num(node.get("x")), "y": _num(node.get("y")),
        "relative_position_id": _scalar(node.get("relative_position_id")),
        "cost": _num(node.get("cost")),
        "prerequisites": _prereq_groups(node),
        "mutually_exclusive": [v for k, v in (node.get("mutually_exclusive").entries
                               if isinstance(node.get("mutually_exclusive"), Node) else [])
                               if k.lower() == "focus" and isinstance(v, str)],
        "has_available": node.get("available") is not None,
        "has_bypass": node.get("bypass") is not None,
        "has_completion_reward": node.get("completion_reward") is not None,
        "available_if_capitulated": _scalar(node.get("available_if_capitulated")) == "yes",
        "search_filters": [v for k, v in (node.get("search_filters").entries
                           if isinstance(node.get("search_filters"), Node) else [])
                           if k == "" and isinstance(v, str)],
        "loc": {"title": title, "desc": desc},
        "source_file": rel,
    }


def parse_focus_defs(layers, loc) -> dict:
    trees: dict[str, dict] = {}
    focuses: dict[str, dict] = {}
    shared: dict[str, dict] = {}
    joint: dict[str, dict] = {}
    styles: dict[str, dict] = {}
    collisions = []
    ordered = R.ordered_files(layers, "common/national_focus", (".txt",))
    for rel, abspath in ordered:
        try:
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[focus_def] WARN {rel}: {e}", file=sys.stderr)
            continue
        for k, v in root.entries:
            kl = k.lower()
            if kl == "focus_tree" and isinstance(v, Node):
                tid = _scalar(v.get("id")) or f"<anon:{rel}>"
                tree_focuses = []
                for fk, fv in v.entries:
                    if fk.lower() == "focus" and isinstance(fv, Node):
                        rec = parse_focus(fv, loc, rel)
                        if rec:
                            rec["kind"] = "focus"
                            tree_focuses.append(rec["id"])
                            if rec["id"] in focuses and focuses[rec["id"]]["source_file"] != rel:
                                collisions.append({"id": rec["id"],
                                                   "first": focuses[rec["id"]]["source_file"],
                                                   "override": rel})
                            focuses[rec["id"]] = rec
                country = None
                cblk = v.get("country")
                if isinstance(cblk, Node):
                    # country = { factor=... modifier{ tag=X } } — record tag refs
                    country = [vv for kk, vv in cblk.entries
                               if kk.lower() == "tag" and isinstance(vv, str)]
                trees[tid] = {
                    "id": tid,
                    "default": _scalar(v.get("default")) == "yes",
                    "country_tags": country,
                    "focus_count": len(tree_focuses),
                    "focuses": tree_focuses,
                    "shared_focus_refs": [vv for kk, vv in v.entries
                                          if kk.lower() == "shared_focus" and isinstance(vv, str)],
                    "source_file": rel,
                }
            elif kl in ("shared_focus", "joint_focus") and isinstance(v, Node):
                rec = parse_focus(v, loc, rel)
                if rec:
                    rec["kind"] = kl
                    if kl == "shared_focus":
                        shared[rec["id"]] = rec
                    else:
                        joint[rec["id"]] = rec
                    focuses[rec["id"]] = rec
            elif kl == "style" and isinstance(v, Node):
                name = _scalar(v.get("name"))
                if name:
                    styles[name] = {
                        "name": name,
                        "default": _scalar(v.get("default")) == "yes",
                        "unavailable": _scalar(v.get("unavailable")),
                        "available": _scalar(v.get("available")),
                        "current": _scalar(v.get("current")),
                        "completed": _scalar(v.get("completed")),
                        "source_file": rel,
                    }
    default_styles = [name for name, style in styles.items() if style["default"]]
    default_style = default_styles[0] if len(default_styles) == 1 else None
    unresolved_styles = []
    for rec in focuses.values():
        style_name = rec["text_icon"] or default_style
        rec["background_style_name"] = style_name
        rec["background_style"] = styles.get(style_name)
        if style_name not in styles:
            unresolved_styles.append({"focus": rec["id"], "style": style_name})
    return {
        "meta": {
            "tree_count": len(trees),
            "focus_count": len(focuses),
            "shared_focus_count": len(shared),
            "joint_focus_count": len(joint),
            "focus_background_style_count": len(styles),
            "default_focus_background_styles": default_styles,
            "unresolved_focus_background_styles": unresolved_styles,
            "files_parsed": len(ordered),
            "id_collisions_last_wins": collisions,
        },
        "trees": dict(sorted(trees.items())),
        "focuses": dict(sorted(focuses.items())),
        "focus_background_styles": dict(sorted(styles.items())),
    }


def build(vanilla=None, mods=None):
    layers = R.build_layers(vanilla=vanilla, mods=mods)
    loc = R.load_loc(layers)
    return parse_focus_defs(layers, loc)


if __name__ == "__main__":
    out = build()
    outp = Path(__file__).resolve().parent / "out" / "focus_defs.json"
    outp.parent.mkdir(exist_ok=True)
    outp.write_text(json.dumps(out, ensure_ascii=False, indent=1), encoding="utf-8")
    m = out["meta"]
    print(f"focus_defs: {m['tree_count']} trees, {m['focus_count']} focuses "
          f"({m['shared_focus_count']} shared) from {m['files_parsed']} files "
          f"-> {outp.name}")
