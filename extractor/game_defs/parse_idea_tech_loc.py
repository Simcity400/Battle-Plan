#!/usr/bin/env python3
"""IDEA + TECHNOLOGY token -> localisation-key cross-reference parser.

The compact replay stream emits idea and technology TOKEN ids, but those ids are
not always the localisation key the UI should display. This parser reads the
effective mod-over-vanilla definition files and emits the derivable join only:

  * ideas: ``common/ideas/*.txt`` -> idea id, category, optional explicit
    ``name = <loc_key>`` override, picture.
  * technologies: ``common/technologies/*.txt`` -> technology id, folder,
    categories, optional explicit top-level ``name = <loc_key>`` override.

No localisation is guessed from the catalog here. The definition-side rule is:
an explicit scalar ``name`` wins; if ``name`` is absent, the definition id is the
loc key; if ``name`` exists but is not scalar, the token is omitted and counted.

stdlib only (+ resolver, paradox_parser).
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

import resolver as R
from paradox_parser import Node
from clausewitz_source_scan import idea_name_token_kinds, technology_name_token_kinds


_IDEA_CATEGORY_FLAGS = {"designer", "law", "use_list_view", "ledger"}
_LOC_KEY_TOKEN_RE = re.compile(r"^[A-Za-z0-9_.]+$")


def _scalar(v):
    return v if isinstance(v, str) else None


def _field_present(node: Node, field: str) -> bool:
    fl = field.lower()
    return any(k.lower() == fl for k, _v in node.entries if k)


def _is_loc_key_shaped(value: str) -> bool:
    return bool(_LOC_KEY_TOKEN_RE.fullmatch(value))


def _quoted_literal_name(value: str, name_token_kind: str | None) -> str | None:
    if name_token_kind != "str":
        return None
    return None if _is_loc_key_shaped(value) else value


def _explicit_name_or_id(token_id: str, node: Node, name_token_kind: str | None = None):
    """Return (loc_key, explicit_name, omitted_reason, literal_name, loc_rule)."""
    if _field_present(node, "name"):
        name = _scalar(node.get("name"))
        if name:
            literal_name = _quoted_literal_name(name, name_token_kind)
            return name, True, None, literal_name, "explicit_name"
        return None, False, "non_scalar_name", None, None
    return token_id, False, None, None, "id"


def _atoms(node):
    if not isinstance(node, Node):
        return []
    return [v for k, v in node.entries if k == "" and isinstance(v, str)]


def _folder_names(node: Node) -> list[str]:
    out = []
    for folder in node.get_all("folder"):
        if not isinstance(folder, Node):
            continue
        name = _scalar(folder.get("name"))
        if name:
            out.append(name)
    return sorted(set(out))


def _list_field_atoms(node: Node, field: str) -> list[str]:
    out = []
    seen = set()
    wanted = field.lower()
    for key, value in node.entries:
        if key.lower() != wanted:
            continue
        for atom in _atoms(value):
            if atom in seen:
                continue
            seen.add(atom)
            out.append(atom)
    return out


def _source_label(layers, abspath) -> str:
    return layers[R._layer_prio_fn(layers)(abspath)].label


def _parse_ideas(layers):
    ideas: dict[str, dict] = {}
    omitted = []
    collisions = []
    files_parsed = 0
    explicit_names = 0
    ordered = R.ordered_files(layers, "common/ideas", (".txt",))
    for rel, abspath in ordered:
        try:
            name_kinds = idea_name_token_kinds(
                Path(abspath).read_text(encoding="utf-8-sig", errors="replace")
            )
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[idea_tech_loc] WARN ideas {rel}: {e}", file=sys.stderr)
            continue
        ideas_block = root.get("ideas")
        if not isinstance(ideas_block, Node):
            continue
        files_parsed += 1
        label = _source_label(layers, abspath)
        for category, catbody in ideas_block.entries:
            if not isinstance(catbody, Node):
                continue
            for iid, body in catbody.entries:
                if not isinstance(body, Node) or not iid:
                    continue
                if iid.lower() in _IDEA_CATEGORY_FLAGS:
                    continue
                loc_key, explicit_name, reason, literal_name, loc_rule = _explicit_name_or_id(
                    iid, body, name_kinds.get((category, iid))
                )
                if reason:
                    omitted.append({
                        "id": iid,
                        "kind": "idea",
                        "category": category,
                        "reason": reason,
                        "source_layer": label,
                        "source_file": rel,
                    })
                    continue
                rec = {
                    "id": iid,
                    "kind": "idea",
                    "loc_key": loc_key,
                    "loc_rule": loc_rule,
                    "literal_name": literal_name,
                    "name_token_kind": name_kinds.get((category, iid)),
                    "explicit_name": explicit_name,
                    "category": category,
                    "picture": _scalar(body.get("picture")),
                    "source_layer": label,
                    "source_file": rel,
                }
                if explicit_name:
                    explicit_names += 1
                if iid in ideas and ideas[iid]["source_file"] != rel:
                    collisions.append({
                        "id": iid,
                        "first": ideas[iid]["source_file"],
                        "override": rel,
                    })
                ideas[iid] = rec
    return ideas, omitted, collisions, files_parsed, explicit_names


def _parse_technologies(layers):
    techs: dict[str, dict] = {}
    omitted = []
    collisions = []
    files_parsed = 0
    explicit_names = 0
    ordered = R.ordered_files(layers, "common/technologies", (".txt",))
    for rel, abspath in ordered:
        try:
            name_kinds = technology_name_token_kinds(
                Path(abspath).read_text(encoding="utf-8-sig", errors="replace")
            )
            root = R.parse_file(abspath)
        except Exception as e:
            print(f"[idea_tech_loc] WARN technologies {rel}: {e}", file=sys.stderr)
            continue
        block = root.get("technologies")
        if not isinstance(block, Node):
            continue
        files_parsed += 1
        label = _source_label(layers, abspath)
        for tid, body in block.entries:
            if tid.startswith("@") or not isinstance(body, Node):
                continue
            loc_key, explicit_name, reason, literal_name, loc_rule = _explicit_name_or_id(
                tid, body, name_kinds.get(tid)
            )
            if reason:
                omitted.append({
                    "id": tid,
                    "kind": "tech",
                    "reason": reason,
                    "source_layer": label,
                    "source_file": rel,
                })
                continue
            rec = {
                "id": tid,
                "kind": "tech",
                "loc_key": loc_key,
                "loc_rule": loc_rule,
                "literal_name": literal_name,
                "name_token_kind": name_kinds.get(tid),
                "explicit_name": explicit_name,
                "categories": _atoms(body.get("categories")),
                "folders": _folder_names(body),
                "desc_loc_key": _scalar(body.get("desc")),
                "enable_equipments": _list_field_atoms(body, "enable_equipments"),
                "enable_equipment_modules": _list_field_atoms(body, "enable_equipment_modules"),
                "source_layer": label,
                "source_file": rel,
            }
            if explicit_name:
                explicit_names += 1
            if tid in techs and techs[tid]["source_file"] != rel:
                collisions.append({
                    "id": tid,
                    "first": techs[tid]["source_file"],
                    "override": rel,
                })
            techs[tid] = rec
    return techs, omitted, collisions, files_parsed, explicit_names


def parse_idea_tech_loc_keys(layers) -> dict:
    ideas, idea_omitted, idea_collisions, idea_files, idea_explicit = _parse_ideas(layers)
    techs, tech_omitted, tech_collisions, tech_files, tech_explicit = _parse_technologies(layers)
    return {
        "meta": {
            "idea_count": len(ideas) + len(idea_omitted),
            "idea_mapped": len(ideas),
            "idea_omitted_no_loc_key": len(idea_omitted),
            "idea_explicit_names": idea_explicit,
            "idea_files_parsed": idea_files,
            "tech_count": len(techs) + len(tech_omitted),
            "tech_mapped": len(techs),
            "tech_omitted_no_loc_key": len(tech_omitted),
            "tech_explicit_names": tech_explicit,
            "tech_files_parsed": tech_files,
            "id_collisions_last_wins": {
                "ideas": idea_collisions,
                "technologies": tech_collisions,
            },
        },
        "tokens": {
            "ideas": dict(sorted(ideas.items())),
            "technologies": dict(sorted(techs.items())),
        },
        "omitted": {
            "ideas": sorted(idea_omitted, key=lambda r: r["id"]),
            "technologies": sorted(tech_omitted, key=lambda r: r["id"]),
        },
    }


def build(vanilla=None, mods=None):
    layers = R.build_layers(vanilla=vanilla, mods=mods)
    return parse_idea_tech_loc_keys(layers)


if __name__ == "__main__":
    out = build()
    outp = Path(__file__).resolve().parent / "out" / "idea_tech_loc_keys.json"
    outp.parent.mkdir(exist_ok=True)
    outp.write_text(json.dumps(out, ensure_ascii=False, indent=1), encoding="utf-8")
    m = out["meta"]
    print(
        f"idea_tech_loc_keys: {m['idea_mapped']}/{m['idea_count']} ideas "
        f"({m['idea_explicit_names']} explicit names), "
        f"{m['tech_mapped']}/{m['tech_count']} techs "
        f"({m['tech_explicit_names']} explicit names) -> {outp.name}"
    )
