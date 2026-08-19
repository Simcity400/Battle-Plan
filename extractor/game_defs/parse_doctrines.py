#!/usr/bin/env python3
"""DOCTRINE catalog parser  (bucket-(c): doctrine_tree view, current 1.19 system).

The 1.19 doctrine system lives under ``common/doctrines/`` (NOT in
``common/technologies`` in this tree) as four pieces:
  folders/        : the doctrine tabs (land/naval/air/special_forces) -> ledger,
                    gfx, name.
  grand_doctrines/: the top-level grand doctrine choices -> folder, name, icon,
                    xp_cost/type, the ``tracks`` they unlock, EFFECT rows. Grand
                    doctrines sharing a folder are MUTUALLY EXCLUSIVE (pick one).
  tracks/         : the milestone tracks (the COLUMNS) -> name, icon, mastery.
  subdoctrines/   : the actual unlockable nodes (the ROWS within a track) ->
                    track, name, xp_cost/type, available, EFFECT rows + ``rewards``
                    (the on-hover effects), and explicit ``xor`` / ``replaces``
                    mutual exclusion where present.

RENDERABILITY (Codex-verified, this revision):
The common/doctrines/*.txt defs carry NO per-node x/y -- the doctrine view is a
GRID (interface/doctrines/doctrines_view.gui ``tracks_grid``, slotsize
424x764px) whose geometry is DERIVED from structure: a subdoctrine's column = the
index of its track in the parent grand doctrine's ``tracks`` list; its row = its
declaration order within that track. We compute a renderable ``position`` (col,
row, grid px) for every doctrine node, preserving SOURCE DECLARATION ORDER (the
prior parser sorted by id and lost it, so it could not derive geometry).

MUTUAL EXCLUSION (Codex-verified):
  * ``xor`` / ``XOR`` on a subdoctrine -> explicit symmetric mutual exclusion
    (e.g. special-forces mountaineers_1 xor mountaineers_2). The prior parser
    swept this into ``effects`` -- we now extract it structurally.
  * ``replaces`` -> a directed replacement/obsolescence edge (NOT symmetric XOR).
  * ``mutually_exclusive`` -> supported if present.
  * grand doctrines in the same folder -> an implicit one-of (XOR) group.

Mod-over-vanilla resolved by effective file set; declaration order preserved.

stdlib only.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import resolver as R
from paradox_parser import Node

_STRUCT = {
    "folder", "track", "name", "description", "icon", "icon_frame",
    "available", "xp_cost", "xp_type", "ai_will_do", "tracks", "rewards",
    "background", "mastery", "allowed", "ledger", "ledger_gfx", "tab_gfx",
    "color_frame", "sound", "replaces", "visible", "doctrine", "id",
    "on_research_complete", "categories", "icon_milestone",
    "xor", "mutually_exclusive", "milestones",
}

_TRACK_COL_W = 424
_TRACK_ROW_H = 96


def _scalar(v):
    return v if isinstance(v, str) else None


def _atoms(node):
    if not isinstance(node, Node):
        return []
    return [v for k, v in node.entries if k == "" and isinstance(v, str)]


def _tracks_of(body: Node):
    """A subdoctrine ``track`` is usually a scalar (one track) but may be a BLOCK
    ``track = { special_forces_first special_forces_second }`` (the special-forces
    nodes that live on two tracks). Return the list of track names (the prior
    parser only read the scalar form -> 8 SF nodes lost their track)."""
    t = body.get("track")
    if isinstance(t, str):
        return [t]
    if isinstance(t, Node):
        return _atoms(t)
    return []


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


def _effects(node: Node):
    out = {}
    for k, v in node.entries:
        if not k or k.lower() in _STRUCT:
            continue
        out[k] = _plain(v)
    rw = node.get("rewards")
    if isinstance(rw, Node):
        out["_rewards"] = {k: _plain(v) for k, v in rw.entries if k}
    return out


def _parse_dir(layers, subdir, kind, by_id, order, collisions):
    for rel, abspath in R.ordered_files(layers, subdir, (".txt",)):
        try:
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[doctrines] WARN {rel}: {e}", file=sys.stderr)
            continue
        for nid, body in root.entries:
            if not isinstance(body, Node) or not nid:
                continue
            rec = {
                "id": nid,
                "kind": kind,
                "name": _scalar(body.get("name")),
                "description": _scalar(body.get("description")),
                "folder": _scalar(body.get("folder")),
                "tracks_member": _tracks_of(body),
                "track": (_tracks_of(body) or [None])[0],
                "icon": _scalar(body.get("icon")),
                "xp_cost": body.get("xp_cost"),
                "xp_type": _scalar(body.get("xp_type")),
                "tracks": _atoms(body.get("tracks")),
                "xor": sorted(set(
                    t for blk in body.get_all("xor") for t in _atoms(blk))),
                "mutually_exclusive": sorted(set(
                    t for blk in body.get_all("mutually_exclusive")
                    for t in _atoms(blk))),
                "replaces": (_atoms(body.get("replaces"))
                             if isinstance(body.get("replaces"), Node)
                             else ([body.get("replaces")]
                                   if isinstance(body.get("replaces"), str)
                                   else [])),
                "position": None,
                "positions": None,
                "effects": _effects(body),
                "source_file": rel,
                "_decl_order": len(order),
            }
            if nid in by_id and by_id[nid]["source_file"] != rel:
                collisions.append({"id": nid, "kind": kind,
                                   "first": by_id[nid]["source_file"],
                                   "override": rel})
            by_id[nid] = rec
            order.append(nid)


def _resolve_doctrine_geometry(by_id: dict):
    """SECOND PASS: derive a renderable grid ``position`` for every node + the XOR
    relationship graph.

    Layout: folder -> grand doctrine(s) -> tracks (COLUMNS) -> subdoctrines (ROWS).
      * a TRACK's column index = its position in the union of grand-doctrine
        ``tracks`` lists for its folder (deterministic, first-seen order).
      * a SUBDOCTRINE's column = its (first) track's column; its ROW = its order
        among the subdoctrines that share that track (declaration order).
      * a GRAND DOCTRINE renders as a header above its track columns.
    Returns a stats dict + fills implicit grand-doctrine XOR groups.
    """
    grands = {n: r for n, r in by_id.items() if r["kind"] == "grand_doctrine"}
    tracks = {n: r for n, r in by_id.items() if r["kind"] == "track"}
    subs = {n: r for n, r in by_id.items() if r["kind"] == "subdoctrine"}

    track_folder: dict[str, str] = {}
    folder_track_order: dict[str, list] = {}
    for gid, g in sorted(grands.items(), key=lambda kv: kv[1]["_decl_order"]):
        fol = g["folder"]
        for tr in g["tracks"]:
            track_folder.setdefault(tr, fol)
            col_list = folder_track_order.setdefault(fol, [])
            if tr not in col_list:
                col_list.append(tr)
    track_col: dict[str, int] = {}
    for fol, cols in folder_track_order.items():
        for i, tr in enumerate(cols):
            track_col[tr] = i

    rows_seen: dict[str, int] = {}

    def _place(track):
        col = track_col.get(track)
        fol = track_folder.get(track)
        if col is None:
            col = _sf_col(track, track_col)
            fol = fol or _sf_folder(track)
        row = rows_seen.get(track, 0)
        rows_seen[track] = row + 1
        return {
            "col": col, "row": row,
            "x": (col if col is not None else 0) * _TRACK_COL_W,
            "y": row * _TRACK_ROW_H,
            "track": track, "folder": fol,
            "source": "track_grid",
        }

    for sid, s in sorted(subs.items(), key=lambda kv: kv[1]["_decl_order"]):
        member = s["tracks_member"] or [None]
        placements = [_place(tr) for tr in member]
        s["positions"] = placements
        s["position"] = placements[0]

    for gid, g in grands.items():
        first_tr = g["tracks"][0] if g["tracks"] else None
        col = track_col.get(first_tr, 0)
        g["position"] = {"col": col, "row": -1,
                         "x": col * _TRACK_COL_W, "y": -_TRACK_ROW_H,
                         "folder": g["folder"], "source": "grand_header"}

    for tid, t in tracks.items():
        col = track_col.get(tid)
        if col is None:
            col = _sf_col(tid, track_col)
        t["position"] = {"col": col, "row": -1,
                         "x": (col if col is not None else 0) * _TRACK_COL_W,
                         "y": -_TRACK_ROW_H,
                         "folder": track_folder.get(tid) or _sf_folder(tid),
                         "source": "track_header"}

    folder_grands: dict[str, list] = {}
    for gid, g in grands.items():
        folder_grands.setdefault(g["folder"], []).append(gid)
    grand_xor_groups = {fol: sorted(ids) for fol, ids in folder_grands.items()
                        if len(ids) > 1}

    explicit_xor = sum(1 for s in subs.values() if s["xor"])

    no_pos = [n for n, r in by_id.items()
              if r["kind"] in ("subdoctrine", "track", "grand_doctrine")
              and r["position"] is None]
    multi_track = sum(1 for s in subs.values()
                      if s.get("positions") and len(s["positions"]) > 1)
    return {
        "subdoctrines_positioned": sum(1 for s in subs.values() if s["position"]),
        "subdoctrine_total": len(subs),
        "multi_track_subdoctrines": multi_track,
        "tracks_positioned": sum(1 for t in tracks.values() if t["position"]),
        "track_total": len(tracks),
        "grand_doctrines_positioned": sum(1 for g in grands.values() if g["position"]),
        "explicit_subdoctrine_xor": explicit_xor,
        "grand_doctrine_xor_groups": grand_xor_groups,
        "unpositioned": sorted(no_pos),
    }


def _sf_col(track, track_col):
    if track is None:
        return None
    if track in track_col:
        return track_col[track]
    base = max(track_col.values(), default=-1) + 1
    order = ["special_forces_first", "special_forces_second"]
    if track in order:
        col = base + order.index(track)
        track_col[track] = col
        return col
    track_col[track] = base
    return base


def _sf_folder(track):
    if track and track.startswith("special_forces"):
        return "special_forces"
    return None


def parse_doctrines(layers) -> dict:
    by_id: dict[str, dict] = {}
    order: list = []
    collisions = []
    for subdir, kind in (
        ("common/doctrines/folders", "folder"),
        ("common/doctrines/grand_doctrines", "grand_doctrine"),
        ("common/doctrines/tracks", "track"),
        ("common/doctrines/subdoctrines", "subdoctrine"),
    ):
        _parse_dir(layers, subdir, kind, by_id, order, collisions)
    geom = _resolve_doctrine_geometry(by_id)
    counts = {}
    for r in by_id.values():
        counts[r["kind"]] = counts.get(r["kind"], 0) + 1
        r.pop("_decl_order", None)
    return {
        "meta": {
            "node_count": len(by_id),
            "by_kind": counts,
            "id_collisions_last_wins": collisions,
            "geometry": geom,
        },
        "doctrines": dict(sorted(by_id.items())),
    }


def build(vanilla=None, mods=None):
    layers = R.build_layers(vanilla=vanilla, mods=mods)
    return parse_doctrines(layers)


if __name__ == "__main__":
    out = build()
    outp = Path(__file__).resolve().parent / "out" / "doctrines.json"
    outp.parent.mkdir(exist_ok=True)
    outp.write_text(json.dumps(out, ensure_ascii=False, indent=1), encoding="utf-8")
    m = out["meta"]
    g = m["geometry"]
    print(f"doctrines: {m['node_count']} nodes {m['by_kind']} -> {outp.name}")
    print(f"  positioned subdoctrines: {g['subdoctrines_positioned']}/"
          f"{g['subdoctrine_total']}, tracks {g['tracks_positioned']}/"
          f"{g['track_total']}, explicit XOR edges: {g['explicit_subdoctrine_xor']}, "
          f"grand-doctrine XOR groups: {list(g['grand_doctrine_xor_groups'])}, "
          f"unpositioned: {len(g['unpositioned'])}")
