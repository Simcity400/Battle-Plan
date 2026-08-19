#!/usr/bin/env python3
"""IDEA + TECHNOLOGY token -> localisation-key cross-reference parser.

The compact replay stream emits idea and technology TOKEN ids, but those ids are
not always the localisation key the UI should display. This parser reads the
effective mod-over-vanilla definition files and emits the derivable join only:

  * ideas: ``common/ideas/*.txt`` -> idea id, category, optional explicit
    ``name = <loc_key>`` override, picture.
  * technologies: ``common/technologies/*.txt`` -> technology id, folder,
    categories, optional explicit top-level ``name = <loc_key>`` override, and
    the authored ``start_year`` when the definition declares one.

``start_year`` is the technology's authored template year, and the replay
consumer authenticates the engine's ahead-of-time quartet against it
(``game_ui/src/research_observer.rs``): a producer that derived the quartet from
the WRONG year -- including the engine's wrapped ``IMUL R9D`` intermediate --
reproduces internally but disagrees with the authored year. A technology that
declares no ``start_year`` carries none here: the key is absent, never 0, so
"this technology authors no start year" can never be read as "year zero".

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
    collect_at_var_defs,
    first_scalar,
    put_present,
    resolve_at_var_scalar,
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


_ENGINE_HOURS_PER_YEAR = 8760
_ENGINE_YEAR_MAX = (2 ** 31 - 1) // _ENGINE_HOURS_PER_YEAR
_ENGINE_YEAR_MIN = -_ENGINE_YEAR_MAX


def _is_whole_number(value) -> bool:
    if isinstance(value, bool):
        return False
    if isinstance(value, int):
        return True
    return isinstance(value, float) and value == value and value not in (
        float("inf"),
        float("-inf"),
    ) and value.is_integer()


def _whole_year(value):
    """The authored value as an engine year, or ``None`` if it is not one.

    ``1936.0`` IS a whole year -- the Clausewitz tokenizer spells it as a float
    and the engine reads an integer -- so it resolves; ``1936.5`` does not.
    """
    if not _is_whole_number(value):
        return None
    year = int(value)
    return year if _ENGINE_YEAR_MIN <= year <= _ENGINE_YEAR_MAX else None


def _start_year(node: Node, at_vars: dict):
    """Return ``(start_year, unresolved_reason, declarations)`` for one technology.

    The engine's template year is a whole year -- its ahead-of-time arithmetic
    multiplies it as an i32 -- so only an integer, literal or an ``@ref``
    defined in scope, becomes a value. Everything else yields ``None`` with a
    reason that is ACCOUNTED rather than guessed at.

    Vanilla really does declare the field twice on one technology
    (``AST_war_carriers_tech`` in ``common/technologies/support.txt``, both
    ``1936``), so a repeat is not automatically a refusal: when every
    declaration resolves to the SAME integer the authored year is unambiguous
    whichever one the engine took, and that is a proof rather than a guess.
    Declarations that DISAGREE are refused, because choosing between them would
    need the engine's own duplicate-key rule and nothing here has it.

    Only the integer is emitted -- no raw spelling. A year is exact in JSON, so
    a raw token would add a second, ambiguous copy (which of two declarations?)
    of a value that cannot be lost in the round trip.

    A REFUSAL IS NOT AN ABSENCE, and the caller must keep the two apart: a
    technology that declares nothing authors no start year, while one whose
    declaration cannot be read has an authored year this parser could not
    recover. The second is stamped on the row so the consumer can fail closed
    instead of quietly authenticating against the weaker check.
    """
    values = node.get_all("start_year")
    if not values:
        return None, None, 0
    if any(not isinstance(value, str) for value in values):
        return None, "non_scalar_start_year", len(values)
    resolved = [resolve_at_var_scalar(value, at_vars)[0] for value in values]
    if any(
        value is None and raw.startswith("@") for value, raw in zip(resolved, values)
    ):
        return None, "unresolved_start_year_reference", len(values)
    years = [_whole_year(value) for value in resolved]
    if any(year is None for year in years):
        reason = (
            "start_year_outside_engine_year_domain"
            if all(_is_whole_number(value) for value in resolved)
            else "non_integer_start_year"
        )
        return None, reason, len(values)
    if len(set(years)) != 1:
        return None, "conflicting_start_year", len(values)
    return years[0], None, len(values)


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
    omitted: dict[str, dict] = {}
    seen_source: dict[str, str] = {}
    collisions = []
    files_parsed = 0
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
                if seen_source.get(iid) not in (None, rel):
                    collisions.append({
                        "id": iid,
                        "first": seen_source[iid],
                        "override": rel,
                    })
                seen_source[iid] = rel
                if reason:
                    omitted[iid] = {
                        "id": iid,
                        "kind": "idea",
                        "category": category,
                        "reason": reason,
                        "source_layer": label,
                        "source_file": rel,
                    }
                    ideas.pop(iid, None)
                    continue
                omitted.pop(iid, None)
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
                ideas[iid] = rec
    return (
        ideas,
        sorted(omitted.values(), key=lambda row: row["id"]),
        collisions,
        files_parsed,
        sum(1 for rec in ideas.values() if rec["explicit_name"]),
    )


def _parse_technologies(
    layers,
    *,
    strict: bool,
    warnings: list[str],
    syntax_allowlist_hits: list[dict],
    wrapper_allowlist_hits: list[dict],
):
    techs: dict[str, dict] = {}
    omitted: dict[str, dict] = {}
    start_year_unresolved: dict[str, dict] = {}
    start_year_repeated: dict[str, dict] = {}
    seen_source: dict[str, str] = {}
    collisions = []
    files_parsed = 0
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
        at_vars = collect_at_var_defs(root, block)
        for tid, body in block.entries:
            if tid.startswith("@") or not isinstance(body, Node):
                continue
            loc_key, explicit_name, reason, literal_name, loc_rule = _explicit_name_or_id(
                tid, body, name_kinds.get(tid)
            )
            if seen_source.get(tid) not in (None, rel):
                collisions.append({
                    "id": tid,
                    "first": seen_source[tid],
                    "override": rel,
                })
            seen_source[tid] = rel
            if reason:
                omitted[tid] = {
                    "id": tid,
                    "kind": "tech",
                    "reason": reason,
                    "source_layer": label,
                    "source_file": rel,
                }
                techs.pop(tid, None)
                start_year_unresolved.pop(tid, None)
                start_year_repeated.pop(tid, None)
                continue
            omitted.pop(tid, None)
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
            start_year, start_year_reason, start_year_declarations = _start_year(
                body, at_vars
            )
            if start_year_reason:
                start_year_unresolved[tid] = {
                    "id": tid,
                    "reason": start_year_reason,
                    "declarations": start_year_declarations,
                    "source_layer": label,
                    "source_file": rel,
                }
            else:
                start_year_unresolved.pop(tid, None)
            if start_year is not None and start_year_declarations > 1:
                start_year_repeated[tid] = {
                    "id": tid,
                    "declarations": start_year_declarations,
                    "source_layer": label,
                    "source_file": rel,
                }
            else:
                start_year_repeated.pop(tid, None)
            put_present(rec, "start_year", start_year)
            if start_year_reason:
                rec["start_year_unresolved"] = True
            put_present(rec, "categories", _list_field_atoms(body, "categories"))
            put_present(rec, "folders", _folder_names(body))
            put_present(rec, "desc_loc_key", first_scalar(body, "desc"))
            put_present(rec, "enable_equipments", _list_field_atoms(body, "enable_equipments"))
            put_present(rec, "enable_equipment_modules", _list_field_atoms(body, "enable_equipment_modules"))
            techs[tid] = rec
    return (
        techs,
        sorted(omitted.values(), key=lambda row: row["id"]),
        collisions,
        files_parsed,
        sum(1 for rec in techs.values() if rec["explicit_name"]),
        sorted(start_year_unresolved.values(), key=lambda row: row["id"]),
        sorted(start_year_repeated.values(), key=lambda row: row["id"]),
    )


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
    (
        techs,
        tech_omitted,
        tech_collisions,
        tech_files,
        tech_explicit,
        tech_start_year_unresolved,
        tech_start_year_repeated,
    ) = _parse_technologies(
        layers,
        strict=strict,
        warnings=warnings,
        syntax_allowlist_hits=syntax_allowlist_hits,
        wrapper_allowlist_hits=wrapper_allowlist_hits,
    )
    tech_start_year_mapped = sum(1 for rec in techs.values() if "start_year" in rec)
    tech_start_year_unresolved_rows = sum(
        1 for rec in techs.values() if rec.get("start_year_unresolved")
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
            "tech_start_year_mapped": tech_start_year_mapped,
            "tech_start_year_absent": (
                len(techs) - tech_start_year_mapped - tech_start_year_unresolved_rows
            ),
            "tech_start_year_unresolved_count": tech_start_year_unresolved_rows,
            "tech_start_year_unresolved": tech_start_year_unresolved,
            "tech_start_year_repeated_agreeing": tech_start_year_repeated,
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
        f"({m['tech_explicit_names']} explicit names, "
        f"{m['tech_start_year_mapped']} start years, "
        f"{len(m['tech_start_year_unresolved'])} unresolved) -> {outp.name}"
    )
