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
from paradox_parser import Node, parse
from clausewitz_source_scan import idea_name_token_kinds, technology_name_token_kinds
from static_artifact_common import (
    CLAUSEWITZ_SYNTAX_QUIRK_ALLOWLIST,
    first_scalar,
    put_present,
    source_inventory,
    validate_clausewitz_text,
)


_IDEA_CATEGORY_FLAGS = {"designer", "law", "use_list_view", "ledger"}
_LOC_KEY_TOKEN_RE = re.compile(r"^[A-Za-z0-9_.]+$")
_EXPECTED_WRAPPER_ALLOWLIST = {
    "ideas": {
        "common/ideas/afghanistan.txt": (
            "Mod-layer removed-country override stub intentionally strips "
            "dedicated Afghanistan idea content."
        ),
        "common/ideas/chile.txt": (
            "Mod-layer removed-country override stub intentionally strips "
            "dedicated Chile idea content."
        ),
        "common/ideas/comchina.txt": (
            "Mod-layer removed-country override stub intentionally strips "
            "dedicated communist China idea content."
        ),
        "common/ideas/paraguay.txt": (
            "Mod-layer removed-country override stub intentionally strips "
            "dedicated Paraguay idea content."
        ),
    },
    "technologies": {},
}


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
        names = node.get_all("name")
        if len(names) > 1:
            return None, False, "duplicate_name", None, None
        name = _scalar(names[0]) if names else None
        if name:
            literal_name = _quoted_literal_name(name, name_token_kind)
            return name, True, None, literal_name, "explicit_name"
        return None, False, "non_scalar_name", None, None
    return token_id, False, None, None, "id"


def _atoms(node):
    if not isinstance(node, Node):
        return []
    return [v for k, v in node.entries if k == "" and isinstance(v, str)]


def _folder_names(node: Node) -> list[str] | None:
    if not node.get_all("folder"):
        return None
    out = []
    for folder in node.get_all("folder"):
        if not isinstance(folder, Node):
            continue
        name = first_scalar(folder, "name")
        if name:
            out.append(name)
    return sorted(set(out))


def _list_field_atoms(node: Node, field: str) -> list[str] | None:
    out = []
    seen = set()
    wanted = field.lower()
    present = False
    for key, value in node.entries:
        if key.lower() != wanted:
            continue
        present = True
        for atom in _atoms(value):
            if atom in seen:
                continue
            seen.add(atom)
            out.append(atom)
    return out if present else None


def _source_label(layers, abspath) -> str:
    return layers[R._layer_prio_fn(layers)(abspath)].label


def _wrapper_allowlist_hit(kind: str, rel: str, source_layer: str) -> dict[str, str] | None:
    reason = _EXPECTED_WRAPPER_ALLOWLIST.get(kind, {}).get(rel)
    if not reason:
        return None
    return {
        "kind": kind,
        "source_file": rel,
        "source_layer": source_layer,
        "reason": reason,
    }


def _parse_ideas(
    layers,
    *,
    strict: bool,
    warnings: list[str],
    syntax_allowlist_hits: list[dict],
    wrapper_allowlist_hits: list[dict],
):
    ideas: dict[str, dict] = {}
    omitted = []
    collisions = []
    files_parsed = 0
    explicit_names = 0
    ordered = R.ordered_files(layers, "common/ideas", (".txt",))
    for rel, abspath in ordered:
        try:
            text = R.read_text(abspath)
            allowlist_hit = validate_clausewitz_text(
                text,
                rel,
                allowlist=CLAUSEWITZ_SYNTAX_QUIRK_ALLOWLIST,
            )
            if allowlist_hit:
                syntax_allowlist_hits.append({"kind": "idea", **allowlist_hit})
            name_kinds = idea_name_token_kinds(
                text
            )
            root = parse(text)
        except Exception as e:
            msg = f"ideas {rel}: {e}"
            if strict:
                raise RuntimeError(f"failed to parse idea loc-key file {msg}") from e
            warnings.append(msg)
            print(f"[idea_tech_loc] WARN {msg}", file=sys.stderr)
            continue
        label = _source_label(layers, abspath)
        ideas_block = root.get("ideas")
        if not isinstance(ideas_block, Node):
            hit = _wrapper_allowlist_hit("ideas", rel, label)
            if hit:
                wrapper_allowlist_hits.append(hit)
                files_parsed += 1
                continue
            msg = f"ideas {rel}: missing expected top-level ideas wrapper"
            if strict:
                raise RuntimeError(msg)
            warnings.append(msg)
            print(f"[idea_tech_loc] WARN {msg}", file=sys.stderr)
            continue
        files_parsed += 1
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
                    "source_layer": label,
                    "source_file": rel,
                }
                put_present(rec, "picture", first_scalar(body, "picture"))
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


def _parse_technologies(
    layers,
    *,
    strict: bool,
    warnings: list[str],
    syntax_allowlist_hits: list[dict],
    wrapper_allowlist_hits: list[dict],
):
    techs: dict[str, dict] = {}
    omitted = []
    collisions = []
    files_parsed = 0
    explicit_names = 0
    ordered = R.ordered_files(layers, "common/technologies", (".txt",))
    for rel, abspath in ordered:
        try:
            text = R.read_text(abspath)
            allowlist_hit = validate_clausewitz_text(
                text,
                rel,
                allowlist=CLAUSEWITZ_SYNTAX_QUIRK_ALLOWLIST,
            )
            if allowlist_hit:
                syntax_allowlist_hits.append({"kind": "technology", **allowlist_hit})
            name_kinds = technology_name_token_kinds(
                text
            )
            root = parse(text)
        except Exception as e:
            msg = f"technologies {rel}: {e}"
            if strict:
                raise RuntimeError(f"failed to parse technology loc-key file {msg}") from e
            warnings.append(msg)
            print(f"[idea_tech_loc] WARN {msg}", file=sys.stderr)
            continue
        label = _source_label(layers, abspath)
        block = root.get("technologies")
        if not isinstance(block, Node):
            hit = _wrapper_allowlist_hit("technologies", rel, label)
            if hit:
                wrapper_allowlist_hits.append(hit)
                files_parsed += 1
                continue
            msg = f"technologies {rel}: missing expected top-level technologies wrapper"
            if strict:
                raise RuntimeError(msg)
            warnings.append(msg)
            print(f"[idea_tech_loc] WARN {msg}", file=sys.stderr)
            continue
        files_parsed += 1
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
                "source_layer": label,
                "source_file": rel,
            }
            put_present(rec, "categories", _list_field_atoms(body, "categories"))
            put_present(rec, "folders", _folder_names(body))
            put_present(rec, "desc_loc_key", first_scalar(body, "desc"))
            put_present(rec, "enable_equipments", _list_field_atoms(body, "enable_equipments"))
            put_present(rec, "enable_equipment_modules", _list_field_atoms(body, "enable_equipment_modules"))
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


def parse_idea_tech_loc_keys(layers, *, strict: bool = True) -> dict:
    warnings: list[str] = []
    syntax_allowlist_hits: list[dict] = []
    wrapper_allowlist_hits: list[dict] = []
    ideas, idea_omitted, idea_collisions, idea_files, idea_explicit = _parse_ideas(
        layers,
        strict=strict,
        warnings=warnings,
        syntax_allowlist_hits=syntax_allowlist_hits,
        wrapper_allowlist_hits=wrapper_allowlist_hits,
    )
    techs, tech_omitted, tech_collisions, tech_files, tech_explicit = _parse_technologies(
        layers,
        strict=strict,
        warnings=warnings,
        syntax_allowlist_hits=syntax_allowlist_hits,
        wrapper_allowlist_hits=wrapper_allowlist_hits,
    )
    idea_source_files = R.ordered_files(layers, "common/ideas", (".txt",))
    tech_source_files = R.ordered_files(layers, "common/technologies", (".txt",))
    return {
        "meta": {
            "idea_count": len(ideas) + len(idea_omitted),
            "idea_mapped": len(ideas),
            "idea_omitted_no_loc_key": len(idea_omitted),
            "idea_explicit_names": idea_explicit,
            "idea_files_parsed": idea_files,
            "idea_files_total": len(idea_source_files),
            "tech_count": len(techs) + len(tech_omitted),
            "tech_mapped": len(techs),
            "tech_omitted_no_loc_key": len(tech_omitted),
            "tech_explicit_names": tech_explicit,
            "tech_files_parsed": tech_files,
            "tech_files_total": len(tech_source_files),
            "warnings": warnings,
            "syntax_allowlist_hits": sorted(
                syntax_allowlist_hits,
                key=lambda row: (row["kind"], row["source_file"], row["matched_error"]),
            ),
            "wrapper_allowlist_hits": sorted(
                wrapper_allowlist_hits,
                key=lambda row: (row["kind"], row["source_file"]),
            ),
            "id_collisions_last_wins": {
                "ideas": idea_collisions,
                "technologies": tech_collisions,
            },
        },
        "source_files": {
            "idea_files": source_inventory(idea_source_files),
            "technology_files": source_inventory(tech_source_files),
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
