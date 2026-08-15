#!/usr/bin/env python3
"""TECH-TREE geometry + effects parser  (bucket-(c): tech_tree / doctrine_tree).

Parses the effective mod-over-vanilla ``common/technologies/*.txt`` set into the
node graph the tech-tree view binds: per-tech FOLDER xy position (geometry),
prerequisites + XOR mutual-exclusion (the link/XOR lines), research/XP cost, the
research folder it lives in, what it ENABLES (equipments/subunits/modules), and
the MODIFIER/EFFECT rows it grants (what the tree shows on hover).

RENDERABILITY (Codex-verified, this revision):
Every tech now carries a resolved ``position`` (x/y/folder/source) so the view can
draw it, plus a ``tree_visible`` flag. A tech's on-screen slot is resolved by:
  1. ``folder.position`` block in the tech def  -> position_source="folder".
  2. else it is a VARIANT (sub-)tech listed in a parent's ``sub_technologies`` and
     the parent has a position: inherit the parent's folder position and add the
     fixed sub-technology slot offset (indexed by the child's ``sub_tech_index``,
     else by its order in the parent list). The slot offsets come from the
     ``sub_technology_slot_N`` containers in ``interface/countrytechtreeview.gui``
     (canonical stack x=141, y=1+27*idx; the prototype-local pixel exact value is
     applied relative to the parent rect by the render layer). -> "subtech_of:<p>".
  3. else the tech is HIDDEN: it has no folder and is not a positioned variant. In
     the game these are focus/event/special-project/ability tech FLAGS gated by
     ``allow = { always = no }`` (or empty stubs) and the engine never draws them
     in the main tech tree. We do NOT fabricate coordinates: position=null,
     tree_visible=false, and we record WHY (allow_always_no / empty / orphan_sub).

Geometry note (Codex C2): technology files declare file-scoped ``@year = N``
variables (``@1936 = 2``) used inside ``folder.position.y = @1936``. We resolve
those references to numbers AND keep the raw token, so the drawing layer gets a
real y and unresolved refs warn instead of silently becoming 0.

Effect rows (Codex C3): we use a STRUCTURAL-KEY DENYLIST -- every top-level key
of a tech that isn't a known structural field is treated as a modifier/effect
row (so unknown/DLC modifiers aren't dropped).

stdlib only (+ resolver, paradox_parser).
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import resolver as R
from paradox_parser import Node
from static_artifact_common import collect_at_var_defs, resolve_at_var_scalar

# Structural keys that are NOT modifier/effect rows.
_STRUCTURAL = {
    "enable_equipments", "enable_equipment_modules", "enable_subunits",
    "enable_building", "enable_tactic", "path", "research_cost", "start_year",
    "folder", "categories", "ai_will_do", "doctrine", "xor", "dependencies",
    "show_equipment_icon", "force_use_small_tech_layout", "sub_technologies",
    "desc", "doctrine_name", "allow", "allow_branch", "on_research_complete",
    "on_research_complete_limit", "enable_tactics", "research_cost_coeff",
    "leads_to_tech", "chance", "ai_research_weights", "complete_tooltip",
    "sub_tech_index", "is_special_project_tech", "special_project_specialization",
    "show_effect_as_desc",
    "modifier",  # handled specially (nested modifier block)
}

# Sub-technology slot offsets (px), observed in the ``sub_technology_slot_N``
# containers of interface/countrytechtreeview.gui. The RELATIVE semantics are
# exact: slots stack to the right of the parent icon, and y advances ~27px per
# index (slot0 y=1, slot1 y=28, slot2 y=55..56 in the GUI). The X offset is
# LAYOUT/PROTOTYPE-specific though -- the GUI uses x=141 (default), x=181 (some
# armour/naval boxes), and x=285 (MTG naval) depending on the parent's prototype
# (Codex R2 #1). We therefore emit the canonical x=141 default AND flag the offset
# ``approximate`` + carry ``slot_index`` so the render layer can substitute the
# real per-prototype x against the actual parent rect.
_SUBTECH_SLOT_DX = 141            # canonical/default; layout-specific in the GUI
_SUBTECH_SLOT_DY0 = 1
_SUBTECH_SLOT_STEP = 27           # exact per-index y pitch (observed 27/28px)


def _slot_offset(idx: int):
    """Return (dx, dy) for sub-technology slot ``idx`` (0-based). dy is exact;
    dx is the canonical default (layout-specific in the GUI -- see module note)."""
    return (_SUBTECH_SLOT_DX, _SUBTECH_SLOT_DY0 + _SUBTECH_SLOT_STEP * int(idx))


def _num(v, scvars: dict):
    """Resolve a scalar that may be an @var reference to a number (keep raw)."""
    return resolve_at_var_scalar(v, scvars)


def _parse_folder(blk: Node, scvars: dict):
    if not isinstance(blk, Node):
        return None
    out = {"name": None, "x": None, "y": None, "x_raw": None, "y_raw": None}
    out["name"] = blk.get("name") if isinstance(blk.get("name"), str) else None
    pos = blk.get("position")
    if isinstance(pos, Node):
        x, xraw = _num(pos.get("x"), scvars)
        y, yraw = _num(pos.get("y"), scvars)
        out.update(x=x, y=y, x_raw=xraw, y_raw=yraw)
    return out


def _atoms(node: Node):
    """List of anonymous + scalar tokens in a list block (e.g. enable list)."""
    if not isinstance(node, Node):
        return []
    out = []
    for k, v in node.entries:
        if isinstance(v, str):
            out.append(v if k == "" else f"{k}={v}")
    return out


def _node_to_plain(node, scvars):
    """Recursively flatten a modifier/effect block to a plain dict (resolving
    @vars in scalar values)."""
    if isinstance(node, str):
        val, _ = _num(node, scvars)
        return val if val is not None else node
    if isinstance(node, Node):
        out = {}
        anon = []
        for k, v in node.entries:
            if k == "":
                anon.append(_node_to_plain(v, scvars))
            else:
                out[k] = _node_to_plain(v, scvars)
        if anon and not out:
            return anon
        if anon:
            out["_list"] = anon
        return out
    return node


def _allow_is_always_no(node: Node) -> bool:
    """True if the tech has ``allow = { always = no }`` (engine never offers it in
    the tree -- it is focus/event/special-project granted)."""
    a = node.get("allow")
    if not isinstance(a, Node):
        return False
    av = a.get("always")
    return isinstance(av, str) and av.lower() == "no"


def parse_tech_block(tech_id: str, node: Node, scvars: dict,
                     source_file: str) -> dict:
    rc, rc_raw = _num(node.get("research_cost"), scvars)
    sy, _ = _num(node.get("start_year"), scvars)
    sti, _ = _num(node.get("sub_tech_index"), scvars)
    folder = _parse_folder(node.get("folder"), scvars)
    folders = [_parse_folder(f, scvars) for f in node.get_all("folder")] or None
    rec = {
        "id": tech_id,
        "research_cost": rc,                 # null when the def omits it (NOT
        "research_cost_raw": rc_raw,         # defaulted -- those are hidden flags)
        "start_year": sy,
        "doctrine": isinstance(node.get("doctrine"), str)
        and node.get("doctrine").lower() == "yes",
        "is_special_project_tech": isinstance(node.get("is_special_project_tech"), str)
        and node.get("is_special_project_tech").lower() == "yes",
        "special_project_specialization":
            _atoms(node.get("special_project_specialization")),
        "force_use_small_tech_layout":
            isinstance(node.get("force_use_small_tech_layout"), str)
            and node.get("force_use_small_tech_layout").lower() == "yes",
        "folders": folders,
        "folder": folder,
        "sub_tech_index": int(sti) if isinstance(sti, (int, float)) else None,
        # position is resolved in a SECOND pass once all techs are known so we can
        # inherit a parent's coordinates for variant sub-techs.
        "position": None,
        "tree_visible": None,
        "runtime_positioned": False,   # engine-grid (electronics/nuclear/SP) tech?
        "categories": _atoms(node.get("categories")),
        "sub_technologies": [t for blk in node.get_all("sub_technologies")
                             for t in _atoms(blk)],   # variant-tech leaf links
        "leads_to": [],          # prerequisites graph (forward links via path=)
        "leads_to_edges": [],    # full edges: {to, research_cost_coeff}
        "dependencies": {},      # other-tech prereqs: {tech: weight}
        "xor": [],               # mutually-exclusive techs
        "enables": {
            "equipments": _atoms(node.get("enable_equipments")),
            "modules": _atoms(node.get("enable_equipment_modules")),
            "subunits": _atoms(node.get("enable_subunits")),
            "buildings": _atoms(node.get("enable_building")),
            "tactics": _atoms(node.get("enable_tactic"))
            + _atoms(node.get("enable_tactics")),
        },
        "allow_always_no": _allow_is_always_no(node),
        "modifiers": {},         # effect rows (hover)
        "source_file": source_file,
    }
    # path = { leads_to_tech= research_cost_coeff= } (repeated) -- keep the edge
    for p in node.get_all("path"):
        if isinstance(p, Node):
            lt = p.get("leads_to_tech")
            if isinstance(lt, str):
                rec["leads_to"].append(lt)
                coeff, _ = _num(p.get("research_cost_coeff"), scvars)
                rec["leads_to_edges"].append(
                    {"to": lt, "research_cost_coeff": coeff})
    # also `leads_to_tech` can appear bare
    for lt in node.get_all("leads_to_tech"):
        if isinstance(lt, str):
            rec["leads_to"].append(lt)
            rec["leads_to_edges"].append({"to": lt, "research_cost_coeff": None})
    # dependencies = { other_tech = weight ... } (forced prereqs)
    dep = node.get("dependencies")
    if isinstance(dep, Node):
        for k, v in dep.entries:
            if k and isinstance(v, str):
                val, _ = _num(v, scvars)
                rec["dependencies"][k] = val if val is not None else v
    # XOR = { techA techB } (repeated; get_all is case-insensitive -> xor/XOR)
    for x in node.get_all("XOR"):
        rec["xor"].extend(_atoms(x))
    # modifier = { ... } nested block
    mod = node.get("modifier")
    if isinstance(mod, Node):
        rec["modifiers"].update({k: _node_to_plain(v, scvars)
                                 for k, v in mod.entries if k})
    # DENYLIST: any other top-level scalar/block that's a real modifier/effect row
    for k, v in node.entries:
        kl = k.lower()
        if kl in _STRUCTURAL or kl == "":
            continue
        if kl in rec or kl in ("id",):
            continue
        rec["modifiers"][k] = _node_to_plain(v, scvars)
    rec["leads_to"] = sorted(set(rec["leads_to"]))
    rec["xor"] = sorted(set(rec["xor"]))
    return rec


def _folder_position(rec: dict):
    """Return (folder_name, x, y) from a tech rec's primary folder, or None if it
    has no positioned folder."""
    f = rec.get("folder") or {}
    if f.get("x") is not None and f.get("y") is not None:
        return (f.get("name"), f.get("x"), f.get("y"))
    # fall back to any folder in the list with a position (alt-folder techs)
    for ff in (rec.get("folders") or []):
        if ff.get("x") is not None and ff.get("y") is not None:
            return (ff.get("name"), ff.get("x"), ff.get("y"))
    return None


def _resolve_positions(techs: dict) -> dict:
    """SECOND PASS: give every tech a resolved ``position`` + ``tree_visible``.

    Returns a stats dict. Three sources, in priority order:
      * folder            -- the tech's own folder.position.
      * subtech_of:<p>    -- variant tech in parent <p>'s sub_technologies; inherit
                             parent position + slot offset (by sub_tech_index/order).
      * hidden            -- no positioned source: position=null, tree_visible=false.
    """
    # parent membership: child -> (parent_id, index_in_list)
    child_parent: dict[str, tuple[str, int]] = {}
    for pid, p in techs.items():
        for i, ch in enumerate(p["sub_technologies"]):
            # last parent wins is fine; sub-tech belongs to one parent grid
            child_parent.setdefault(ch, (pid, i))

    stats = {"folder": 0, "subtech": 0, "hidden": 0,
             "hidden_allow_no": 0, "hidden_empty": 0,
             "hidden_orphan_subindex": 0, "hidden_secret_weapon": 0,
             "hidden_ability_stub": 0, "hidden_other": 0}

    # secret-weapons folder categories: the engine flows these into the special
    # electronics folder by category/start_year at RUNTIME -- they carry no static
    # folder.position anywhere on disk (proven in vanilla too).
    _SECRET_CATS = {"electronics", "radar_tech", "rocketry", "jet_technology",
                    "nuclear"}

    # 1) techs with their own folder position
    for tid, rec in techs.items():
        fp = _folder_position(rec)
        if fp:
            name, x, y = fp
            rec["position"] = {"x": x, "y": y, "folder": name,
                               "source": "folder"}
            rec["tree_visible"] = True
            stats["folder"] += 1

    # 2) variant sub-techs inherit parent position + slot offset
    for tid, rec in techs.items():
        if rec["position"] is not None:
            continue
        pe = child_parent.get(tid)
        if not pe:
            continue
        pid, list_idx = pe
        parent = techs.get(pid)
        if not parent or not parent.get("position"):
            continue
        idx = rec["sub_tech_index"]
        if idx is None:
            idx = list_idx
        dx, dy = _slot_offset(idx)
        ppos = parent["position"]
        rec["position"] = {
            "x": ppos["x"] + dx, "y": ppos["y"] + dy,
            "folder": ppos.get("folder"),
            "source": f"subtech_of:{pid}",
            "parent": pid, "slot_index": idx,
            # dy exact; dx is the canonical default (per-prototype in the GUI) ->
            # render layer applies the real parent-box slot x. (Codex R2 #1)
            "slot_offset": {"dx": dx, "dy": dy, "dx_approximate": True},
            "parent_position": {"x": ppos["x"], "y": ppos["y"]},
        }
        rec["tree_visible"] = True
        stats["subtech"] += 1

    # 3) everything left has NO STATIC tree position -- prove WHY, don't fake xy.
    #    Buckets are MOST-SPECIFIC-FIRST so they stay disjoint (Codex R2 #4):
    #    special-project / secret-weapon markers win over a generic allow=no.
    #    ``runtime_positioned`` distinguishes engine-grid techs (electronics /
    #    rocketry / nuclear / special-project facility -- drawn at runtime, just
    #    not from a static folder.position) from truly non-tree focus stubs
    #    (Codex R2 #2): both have position=null, but consumers can tell them apart.
    for tid, rec in techs.items():
        if rec["position"] is not None:
            continue
        cats = set(rec["categories"])
        is_special_project = bool(
            rec["is_special_project_tech"] or rec["special_project_specialization"])
        runtime_positioned = False
        if rec["sub_tech_index"] is not None and tid not in child_parent:
            # declares a slot index but no parent lists it -> orphan variant
            reason = "orphan_sub_tech_index"
            stats["hidden_orphan_subindex"] += 1
        elif is_special_project or (cats & _SECRET_CATS):
            # electronics / rocketry / nuclear secret-weapons + special-project
            # facility techs: the engine flows these into the dedicated runtime
            # grids (electronics/experimental-rockets/atomic-research) by category
            # -- no static folder.position exists on disk (proven in vanilla too).
            reason = "secret_weapon_folder_runtime_positioned"
            runtime_positioned = True
            stats["hidden_secret_weapon"] += 1
        elif rec["allow_always_no"]:
            # allow={always=no}: never offered in the main tree (focus/event-only)
            reason = "allow_always_no"
            stats["hidden_allow_no"] += 1
        elif not rec["modifiers"] and not any(rec["enables"].values()) \
                and rec["research_cost"] is None:
            reason = "empty_stub"
            stats["hidden_empty"] += 1
        elif not any(rec["enables"].values()) and not rec["leads_to"]:
            # tiny effect-only tech granted by a focus/ability (no enables, no path)
            reason = "ability_stub_focus_granted"
            stats["hidden_ability_stub"] += 1
        else:
            reason = "no_folder_focus_or_special_project"
            stats["hidden_other"] += 1
        rec["position"] = None
        # tree_visible=False == "no STATIC renderable position". runtime_positioned
        # marks the engine-grid techs that the game DOES draw (just not statically).
        rec["tree_visible"] = False
        rec["runtime_positioned"] = runtime_positioned
        rec["hidden_reason"] = reason
        stats["hidden"] += 1
    return stats


def _collect_scvars(file_root: Node, tech_block: Node) -> dict:
    """File-scoped @var defs (``@1936 = 2``). They normally live as top entries of
    the ``technologies={}`` block, but some files/mods declare them at the FILE
    top level. Collect from both so geometry refs always resolve (Codex Q2).

    The rule itself lives in ``static_artifact_common`` because
    ``parse_idea_tech_loc.py`` resolves ``start_year`` out of the same files and
    must not carry a second copy of it."""
    # file top-level @defs first (fallback), in-block @defs win the tie.
    return collect_at_var_defs(file_root, tech_block)


def parse_technologies(layers) -> dict:
    ordered = R.ordered_files(layers, "common/technologies", (".txt",))
    techs: dict[str, dict] = {}
    folders: set[str] = set()
    unresolved_refs = set()
    collisions = []
    for rel, abspath in ordered:
        try:
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[tech] WARN {rel}: {e}", file=sys.stderr)
            continue
        block = root.get("technologies")
        if not isinstance(block, Node):
            continue
        scvars = _collect_scvars(root, block)
        for tid, tbody in block.entries:
            if tid.startswith("@") or not isinstance(tbody, Node):
                continue
            rec = parse_tech_block(tid, tbody, scvars, rel)
            if rec["folder"] and rec["folder"].get("name"):
                folders.add(rec["folder"]["name"])
            # track unresolved @var geometry
            f = rec.get("folder") or {}
            for axis in ("x", "y"):
                raw = f.get(axis + "_raw")
                if isinstance(raw, str) and raw.startswith("@") and f.get(axis) is None:
                    unresolved_refs.add(raw)
            if tid in techs and techs[tid]["source_file"] != rel:
                collisions.append({"id": tid, "first": techs[tid]["source_file"],
                                   "override": rel})
            techs[tid] = rec
    pos_stats = _resolve_positions(techs)
    if unresolved_refs:
        print(f"[tech] WARN unresolved @var geometry refs: "
              f"{sorted(unresolved_refs)}", file=sys.stderr)

    no_cost = sorted(t for t, r in techs.items() if r["research_cost"] is None)
    no_pos = sorted(t for t, r in techs.items() if r["position"] is None)
    renderable = sum(1 for r in techs.values() if r["position"] is not None)
    return {
        "meta": {
            "tech_count": len(techs),
            "files_parsed": len(ordered),
            "folders": sorted(folders),
            "doctrine_techs": sum(1 for t in techs.values() if t["doctrine"]),
            "unresolved_geometry_refs": sorted(unresolved_refs),
            "id_collisions_last_wins": collisions,
            # renderability ledger
            "position_stats": pos_stats,
            "renderable": renderable,            # techs with a resolved position
            "tree_visible": sum(1 for r in techs.values() if r["tree_visible"]),
            "hidden_techs": no_pos,              # proven non-tree techs (xy=null)
            "no_research_cost": no_cost,         # techs whose def omits cost
            "no_research_cost_count": len(no_cost),
        },
        "technologies": dict(sorted(techs.items())),
    }


def build(vanilla=None, mods=None):
    layers = R.build_layers(vanilla=vanilla, mods=mods)
    return parse_technologies(layers)


if __name__ == "__main__":
    out = build()
    outp = Path(__file__).resolve().parent / "out" / "technologies.json"
    outp.parent.mkdir(exist_ok=True)
    outp.write_text(json.dumps(out, ensure_ascii=False, indent=1), encoding="utf-8")
    m = out["meta"]
    print(f"technologies: {m['tech_count']} techs from {m['files_parsed']} files, "
          f"{len(m['folders'])} folders, {m['doctrine_techs']} doctrine, "
          f"{m['renderable']} renderable, {len(m['hidden_techs'])} hidden, "
          f"{m['no_research_cost_count']} no-cost, "
          f"{len(m['unresolved_geometry_refs'])} unresolved geom -> {outp.name}")
    print(f"  position sources: {m['position_stats']}")
