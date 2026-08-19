#!/usr/bin/env python3
"""CHARACTERS / ADVISORS / COMMANDERS static-def parser  (bucket-(c):
advisor_role_selection eligibility, officer_corps / unit_leader / navy_leader
roster identity, command_character).

The modern (post-NSB) the game character system lives in ``common/characters/*.txt``.
A character carries any of several role sub-blocks; we catalog them all so the
app can bind the advisor/officer/leader views:

  advisor          : { slot, idea_token, ledger, cost, traits[], allowed-presence }
                     -> the ADVISOR-eligibility def the role-selection view needs
                     (these are the modern advisors NOT present as common/ideas
                     entries — the gap Codex round-2 surfaced).
  country_leader   : { ideology, traits[], expire } -> head-of-state identity
  field_marshal /  : { skill, attack/defense/planning/logistics_skill, traits[] }
  corps_commander     -> the general roster the officer-corps / unit-leader views show
  navy_leader      : { skill, attack/defense/maneuvering/coordination_skill, traits[] }

Per character: id, name, the role flags present, and each role's fields.
``idea_token`` is the join key the live country advisor slots reference.

NOTE: ``common/characters`` IS in the AMSB mod's replace_path, so the mod's
character files fully REPLACE vanilla's at the dir level — handled correctly by
the shared resolver (replace_path drops vanilla chars before adding the mod's).

stdlib only.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import resolver as R
from paradox_parser import Node
from clausewitz_source_scan import character_name_token_kinds


def _scalar(v):
    return v if isinstance(v, str) else None


def _atoms(node):
    if not isinstance(node, Node):
        return []
    return [v for k, v in node.entries if k == "" and isinstance(v, str)]


def _advisor(blk: Node):
    return {
        "slot": _scalar(blk.get("slot")),
        "idea_token": _scalar(blk.get("idea_token")),
        "ledger": _scalar(blk.get("ledger")),
        "cost": blk.get("cost"),
        "traits": _atoms(blk.get("traits")),
        "has_allowed": blk.get("allowed") is not None,
        "has_available": blk.get("available") is not None,
        "can_be_fired": _scalar(blk.get("can_be_fired")),
    }


def _commander(blk: Node):
    return {k: blk.get(k) for k in (
        "skill", "attack_skill", "defense_skill", "planning_skill",
        "logistics_skill", "maneuvering_skill", "coordination_skill")
        if isinstance(blk.get(k), str)} | {"traits": _atoms(blk.get("traits"))}


def _country_leader(blk: Node):
    return {
        "ideology": _scalar(blk.get("ideology")),
        "expire": _scalar(blk.get("expire")),
        "traits": _atoms(blk.get("traits")),
    }


def parse_character(cid: str, node: Node, rel: str, name_token_kind: str | None = None) -> dict:
    rec = {
        "id": cid,
        "name": _scalar(node.get("name")),
        "name_token_kind": name_token_kind,
        "roles": [],
        "source_file": rel,
    }
    adv = node.get("advisor")
    if isinstance(adv, Node):
        rec["roles"].append("advisor")
        rec["advisor"] = _advisor(adv)
    cl = node.get("country_leader")
    if isinstance(cl, Node):
        rec["roles"].append("country_leader")
        rec["country_leader"] = _country_leader(cl)
    for role in ("field_marshal", "corps_commander", "navy_leader"):
        rb = node.get(role)
        if isinstance(rb, Node):
            rec["roles"].append(role)
            rec[role] = _commander(rb)
    return rec


def parse_characters(layers) -> dict:
    chars: dict[str, dict] = {}
    advisors_by_token: dict[str, dict] = {}
    collisions = []
    ordered = R.ordered_files(layers, "common/characters", (".txt",))
    for rel, abspath in ordered:
        try:
            name_token_kinds = character_name_token_kinds(
                Path(abspath).read_text(encoding="utf-8-sig", errors="replace")
            )
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[characters] WARN {rel}: {e}", file=sys.stderr)
            continue
        label = layers[R._layer_prio_fn(layers)(abspath)].label
        block = root.get("characters")
        if not isinstance(block, Node):
            continue
        for cid, body in block.entries:
            if not isinstance(body, Node) or not cid:
                continue
            rec = parse_character(cid, body, rel, name_token_kinds.get(cid))
            rec["source_layer"] = label
            if not rec["roles"]:
                rec["roles"] = []
            if cid in chars and chars[cid]["source_file"] != rel:
                collisions.append({"id": cid, "first": chars[cid]["source_file"],
                                   "override": rel})
            chars[cid] = rec
            adv = rec.get("advisor")
            if adv and adv.get("idea_token"):
                advisors_by_token[adv["idea_token"]] = {
                    "character": cid, "name": rec["name"], **adv,
                    "source_file": rel}

    role_counts = {}
    for c in chars.values():
        for r in c["roles"]:
            role_counts[r] = role_counts.get(r, 0) + 1
    return {
        "meta": {
            "character_count": len(chars),
            "advisor_tokens": len(advisors_by_token),
            "by_role": role_counts,
            "files_parsed": len(ordered),
            "id_collisions_last_wins": collisions,
        },
        "characters": dict(sorted(chars.items())),
        "advisors_by_token": dict(sorted(advisors_by_token.items())),
    }


def build(vanilla=None, mods=None):
    layers = R.build_layers(vanilla=vanilla, mods=mods)
    return parse_characters(layers)


if __name__ == "__main__":
    out = build()
    outp = Path(__file__).resolve().parent / "out" / "characters.json"
    outp.parent.mkdir(exist_ok=True)
    outp.write_text(json.dumps(out, ensure_ascii=False, indent=1), encoding="utf-8")
    m = out["meta"]
    print(f"characters: {m['character_count']} ({m['advisor_tokens']} advisor "
          f"tokens) {m['by_role']} from {m['files_parsed']} files -> {outp.name}")
