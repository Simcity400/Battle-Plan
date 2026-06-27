#!/usr/bin/env python3
"""EVENTS static-def parser  (bucket-(c): event_window / popup_window / alerts).

Parses the effective mod-over-vanilla ``events/*.txt`` set into a structured
catalog the replay UI binds when the diff engine reports a fired-event id (the
diff has the *id*, this supplies the TITLE / DESC / PICTURE / OPTION text + loc).

Per event we extract:
  id                : namespace.N (globally unique; the diff-engine key)
  kind              : country_event | news_event | state_event | unit_leader_event
  title             : {key, text} — loc key + resolved English (color stripped)
  desc              : list of {key, text} (events can have multiple conditional
                      desc={ text= trigger= } blocks; we keep them all, in order)
  picture           : GFX_* sprite name (the event art)
  major             : bool (news/major banner)
  hidden            : bool (hide_window / is_triggered_only-only hidden events)
  fire_only_once    : bool
  is_triggered_only : bool
  options           : list of {key, text, name_key} — each option's button text
                      (loc), plus a short effect/ai hint where trivially present.
  source_file       : the effective file the event came from (mod or vanilla)

OVERRIDE MODEL (verified): the game loads event files alphabetically; the mod dir
loads after vanilla. The unit of override is the FILE relpath — a mod file with
the same stem REPLACES the vanilla file (whole-file), and event ids are globally
unique. We therefore resolve the effective FILE set (mod file wins on same stem),
then parse every effective file; if the same event id appears twice across two
DIFFERENT effective files, last-parsed wins and we record a collision note (this
mirrors the engine's last-loaded-wins for a duplicated id).

stdlib only (+ game_defs.resolver, game_assets.paradox_parser).
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import resolver as R
from paradox_parser import Node


EVENT_KINDS = {
    "country_event", "news_event", "state_event", "unit_leader_event",
    "operative_leader_event",
}


def _scalar(v):
    return v if isinstance(v, str) else None


def _loc(loc, key):
    """Return {key, text} for a loc key (text = color-stripped English)."""
    if not key:
        return None
    return {"key": key, "text": loc.resolve(key) if loc else None}


def _parse_desc(node_or_str, loc):
    """An event ``desc`` is either a bare loc key (str) or a block
    ``{ text=<key> trigger={...} }`` (and there may be several). Return a
    {key,text,conditional} dict."""
    if isinstance(node_or_str, str):
        d = _loc(loc, node_or_str) or {"key": node_or_str, "text": None}
        d["conditional"] = False
        return d
    if isinstance(node_or_str, Node):
        key = _scalar(node_or_str.get("text"))
        d = _loc(loc, key) or {"key": key, "text": None}
        d["conditional"] = node_or_str.get("trigger") is not None
        return d
    return None


def _parse_option(node: Node, loc):
    """One ``option={ name=<key> ... }`` -> {key,text, has_trigger, hidden}."""
    if not isinstance(node, Node):
        return None
    name = node.get("name")
    # name may itself be a {text= trigger=} block (rare conditional button text)
    if isinstance(name, Node):
        name_key = _scalar(name.get("text"))
    else:
        name_key = _scalar(name)
    o = _loc(loc, name_key) or {"key": name_key, "text": None}
    o["has_trigger"] = node.get("trigger") is not None
    o["hidden"] = (_scalar(node.get("ai_chance")) is None
                   and node.get("name") is None)
    return o


def _truthy(v):
    return isinstance(v, str) and v.lower() in ("yes", "true")


def parse_event_block(kind: str, node: Node, loc, source_file: str) -> dict | None:
    if not isinstance(node, Node):
        return None
    eid = _scalar(node.get("id"))
    if not eid:
        return None
    title = node.get("title")
    title_key = _scalar(title.get("text")) if isinstance(title, Node) else _scalar(title)
    rec = {
        "id": eid,
        "kind": kind,
        "title": _loc(loc, title_key),
        "desc": [],
        "picture": _scalar(node.get("picture")),
        "major": _truthy(node.get("major")),
        "hidden": _truthy(node.get("hidden")) or _truthy(node.get("hide_window")),
        "fire_only_once": _truthy(node.get("fire_only_once")),
        "is_triggered_only": _truthy(node.get("is_triggered_only")),
        "options": [],
        "source_file": source_file,
    }
    for d in node.get_all("desc"):
        pd = _parse_desc(d, loc)
        if pd:
            rec["desc"].append(pd)
    for o in node.get_all("option"):
        po = _parse_option(o, loc)
        if po:
            rec["options"].append(po)
    return rec


def parse_events(layers, loc) -> dict:
    events: dict[str, dict] = {}
    namespaces: set[str] = set()
    collisions: list[dict] = []
    files_parsed = 0
    # Stable load order (vanilla before mod, then relpath), path-boundary-safe.
    ordered = R.ordered_files(layers, "events", (".txt",))
    for rel, abspath in ordered:
        try:
            root = R.parse_file(abspath)
        except Exception as e:  # never let one bad file kill the catalog
            print(f"[events] WARN parse failed {rel}: {e}", file=sys.stderr)
            continue
        files_parsed += 1
        for key, val in root.entries:
            kl = key.lower()
            if kl == "add_namespace" and isinstance(val, str):
                namespaces.add(val)
            elif kl in EVENT_KINDS and isinstance(val, Node):
                rec = parse_event_block(kl, val, loc, rel)
                if not rec:
                    continue
                if rec["id"] in events and events[rec["id"]]["source_file"] != rel:
                    collisions.append({
                        "id": rec["id"],
                        "first": events[rec["id"]]["source_file"],
                        "override": rel,
                    })
                events[rec["id"]] = rec  # last-parsed (mod after vanilla) wins
    return {
        "meta": {
            "files_parsed": files_parsed,
            "event_count": len(events),
            "namespaces": sorted(namespaces),
            "id_collisions_last_wins": collisions,
        },
        "events": dict(sorted(events.items())),
    }


def build(vanilla=None, mods=None):
    layers = R.build_layers(vanilla=vanilla, mods=mods)
    loc = R.load_loc(layers)
    return parse_events(layers, loc)


if __name__ == "__main__":
    out = build()
    outp = Path(__file__).resolve().parent / "out" / "events.json"
    outp.parent.mkdir(exist_ok=True)
    outp.write_text(json.dumps(out, ensure_ascii=False, indent=1), encoding="utf-8")
    m = out["meta"]
    print(f"events: {m['event_count']} from {m['files_parsed']} files, "
          f"{len(m['namespaces'])} namespaces, "
          f"{len(m['id_collisions_last_wins'])} id-collisions -> {outp.name}")
