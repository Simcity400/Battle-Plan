#!/usr/bin/env python3
"""the game save -> DIPLOMACY & INTEL oracle (``diplomacy.json``).

Deep-layer extension of the save oracle. It reconstructs the diplomatic graph
from the per-country relation entries plus the top-level faction blocks.

Top-level output:
  - ``factions``   : list of {id, ideology, icon, members[...], leader (=first
                     member), template, leader_change_date}.
  - ``wars``       : de-duplicated pairwise wars derived from ``war_relation``,
                     each {a, b, start_date, a_casualties, b_casualties,
                     war_score_a_vs_b, war_score_b_vs_a, captured_provinces_*}.
  - ``subjects``   : list of {overlord, subject, autonomy_state, value,
                     start_date} from ``puppet`` relations.

Per country (``countries.<TAG>``):
  - ``faction``        : the faction TAG belongs to (back-reference), or null.
  - ``relations``      : {OTHER: {attitude, opinion_sum, opinion_modifiers[...],
                         guarantee, non_aggression_pact, military_access,
                         embargo, at_war}} — flags present as booleans/objects.
  - ``wargoals``       : list of {type, target, states[...], expire}.
  - ``operatives``     : from intelligence_agency: agency name + list of
                         {name, skill, experience, traits[...], state,
                         nationalities[...]}.
  - ``intel_levels``   : NOTE: per-country intel is a POSITIONAL array in the
                         save (countries.<TAG>.intel.intel = { {civ army navy
                         air} ... }) keyed by internal country index, not TAG —
                         so we report the count and leave the positional decode
                         to the live layer (see REALITY NOTE).

Key paths (verified against a real 1.19 melt):
  faction = { id icon ideology members={"GER" "ITA" ...} template
              faction_leader_change_date }                  (TOP-LEVEL, repeated)
  countries.<TAG>.diplomacy.active_relations.<OTHER> = {
      cached_sum attitude opinion={modifier date value} guarantee={first second}
      non_aggression_pact military_access embargo war_relation={...} puppet={...} }
  countries.<TAG>.diplomacy.wargoals.<type> = {
      wargoaldata_actor wargoaldata_recipient states={..} expire }
  countries.<TAG>.intelligence_agency = { name operative={ name skill experience
      traits state nationalities } ... }
  countries.<TAG>.intel.intel = { {civilian army navy airforce} ... }  POSITIONAL

REALITY NOTES:
  - No top-level ``war={}`` section exists; wars are pairwise ``war_relation``
    entries inside relations, so we collapse the two directions into one war.
  - ``intel.intel`` is a positional array indexed by the engine's internal
    country index, NOT by tag. Resolving the index->tag map is a live-memory /
    cross-section join we do not attempt here; we report the raw positional rows
    count so the data is acknowledged, not invented.
  - Strategic intel NETWORKS (top-level strategic_operatives) are graph blobs;
    we capture each operative's target + state coverage size, not the full graph.

stdlib only. Reuses the shared tokenizer via ``deep_common``.
"""

from __future__ import annotations

from decimal import Decimal, InvalidOperation

import deep_common as dc
from deep_common import (
    LBRACE, atom_value, iter_block_entries, _skip_block_body,
    to_int, to_float, read_scalar_list,
)


# ---------------------------------------------------------------------------
# Top-level factions
# ---------------------------------------------------------------------------

def _parse_faction(tz, factions):
    rec = {"id": None, "ideology": None, "icon": None, "template": None,
           "members": [], "leader": None, "leader_change_date": None}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "members":
                rec["members"] = read_scalar_list(tz)
            elif key == "id":
                _id = None
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        _skip_block_body(tz)
                    elif k == "id":
                        _id = to_int(atom_value(op))
                rec["id"] = _id
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "ideology":
            rec["ideology"] = v
        elif key == "icon":
            rec["icon"] = v
        elif key == "template":
            rec["template"] = v
        elif key == "faction_leader_change_date":
            rec["leader_change_date"] = v
    if rec["members"]:
        rec["leader"] = rec["members"][0]
    factions.append(rec)


# ---------------------------------------------------------------------------
# Per-country relations / wargoals / operatives
# ---------------------------------------------------------------------------

def _parse_war_relation(tz):
    rec = {"start_date": None, "first": None, "second": None,
           "first_casualties": None, "second_casualties": None,
           "score_first_vs_second": None, "score_second_vs_first": None,
           # B5 residuals.  Keep the source spelling as well as the exact
           # fixed-5 integer so a live-memory probe never has to compare an
           # IEEE-754 approximation with the save oracle.
           "threat": None, "threat_text": None,
           "threat_fixed5_raw": None,
           "first_was_instigator": None,
           "hostility_reason_instigator": None,
           "hostility_reason_defender": None}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            if k in ("war_score_first_vs_second", "war_score_second_vs_first"):
                total = None
                for kk, oo in iter_block_entries(tz):
                    if oo is LBRACE:
                        _skip_block_body(tz)
                        continue
                    vv = atom_value(oo)
                    # province_capture is the dominant war-score component
                    if kk == "province_capture":
                        total = to_float(vv)
                key = ("score_first_vs_second"
                       if k == "war_score_first_vs_second"
                       else "score_second_vs_first")
                rec[key] = total
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "first":
            rec["first"] = v
        elif k == "second":
            rec["second"] = v
        elif k == "start_date":
            rec["start_date"] = v
        elif k == "first_casualties":
            rec["first_casualties"] = to_int(v)
        elif k == "second_casualties":
            rec["second_casualties"] = to_int(v)
        elif k == "threat":
            rec["threat_text"] = v
            rec["threat"] = to_float(v)
            try:
                scaled = Decimal(v) * 100_000
                if scaled == scaled.to_integral_value():
                    rec["threat_fixed5_raw"] = int(scaled)
            except InvalidOperation:
                pass
        elif k == "first_was_instigator":
            if v == "yes":
                rec["first_was_instigator"] = True
            elif v == "no":
                rec["first_was_instigator"] = False
        elif k == "hostility_reason_instigator":
            rec["hostility_reason_instigator"] = v
        elif k == "hostility_reason_defender":
            rec["hostility_reason_defender"] = v
    return rec


def _parse_puppet(tz):
    rec = {"autonomy_state": None, "value": None,
           "first": None, "second": None, "start_date": None}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "autonomy_state":
            rec["autonomy_state"] = v
        elif k == "value":
            rec["value"] = to_float(v)
        elif k == "first":
            rec["first"] = v
        elif k == "second":
            rec["second"] = v
        elif k == "start_date":
            rec["start_date"] = v
    return rec


def _parse_relation(tz, wars, subjects, owner_tag, other_tag):
    rel = {"attitude": None, "opinion_sum": None,
           "opinion_modifiers": [], "guarantee": False,
           "non_aggression_pact": False, "military_access": False,
           "embargo": False, "at_war": False}
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "opinion":
                mod = None
                val = None
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        _skip_block_body(tz)
                        continue
                    v = atom_value(op)
                    if k == "modifier":
                        mod = v
                    elif k == "value":
                        val = to_float(v)
                rel["opinion_modifiers"].append({"modifier": mod, "value": val})
            elif key == "guarantee":
                rel["guarantee"] = True
                _skip_block_body(tz)
            elif key == "non_aggression_pact":
                rel["non_aggression_pact"] = True
                _skip_block_body(tz)
            elif key == "military_access":
                rel["military_access"] = True
                _skip_block_body(tz)
            elif key == "embargo":
                rel["embargo"] = True
                _skip_block_body(tz)
            elif key == "war_relation":
                rel["at_war"] = True
                w = _parse_war_relation(tz)
                # Preserve the traversal identity.  `wars` is later collapsed
                # for the legacy product view, while `war_relation_rows` keeps
                # every ordered save row for B5 all-row acceptance.
                w["owner"] = owner_tag
                w["relation_other"] = other_tag
                wars.append(w)
            elif key == "puppet":
                p = _parse_puppet(tz)
                subjects.append(p)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "attitude":
            rel["attitude"] = v
        elif key == "cached_sum":
            rel["opinion_sum"] = to_int(v)
    return rel


def _parse_active_relations(tz, rels, wars, subjects, owner_tag):
    for other, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        rels[other] = _parse_relation(tz, wars, subjects, owner_tag, other)


def _parse_wargoals(tz):
    out = []
    for wtype, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        rec = {"type": wtype, "target": None, "states": [], "expire": None}
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                if k == "states":
                    rec["states"] = [to_int(s) for s in read_scalar_list(tz)]
                else:
                    _skip_block_body(tz)
                continue
            v = atom_value(op)
            if v is None:
                continue
            if k == "wargoaldata_recipient":
                rec["target"] = v
            elif k == "expire":
                rec["expire"] = v
        out.append(rec)
    return out


def _parse_diplomacy(tz, rec, wars, subjects, owner_tag):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "active_relations":
                _parse_active_relations(tz, rec["relations"], wars, subjects,
                                        owner_tag)
            elif key == "wargoals":
                rec["wargoals"] = _parse_wargoals(tz)
            else:
                _skip_block_body(tz)
            continue
        # scalars (faction_join_date, naval_blockade) ignored


def _parse_intelligence_agency(tz, rec):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "operative":
                op_rec = {"name": None, "skill": None, "experience": None,
                          "traits": [], "state": None, "nationalities": []}
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        if k == "traits":
                            op_rec["traits"] = read_scalar_list(tz)
                        elif k == "nationalities":
                            op_rec["nationalities"] = read_scalar_list(tz)
                        else:
                            _skip_block_body(tz)
                        continue
                    v = atom_value(op)
                    if v is None:
                        continue
                    if k == "name":
                        op_rec["name"] = v
                    elif k == "skill":
                        op_rec["skill"] = to_int(v)
                    elif k == "experience":
                        op_rec["experience"] = to_float(v)
                    elif k == "state":
                        op_rec["state"] = v
                rec["operatives"].append(op_rec)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "name":
            rec["agency_name"] = v


def _parse_intel(tz, rec):
    """intel={ intel={ { civ army navy air } ... } ... } — POSITIONAL array."""
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "intel":
                # count the positional rows
                rows = 0
                for k, op in iter_block_entries(tz):
                    if op is LBRACE:
                        _skip_block_body(tz)
                        rows += 1
                    elif op is None:
                        rows += 1
                rec["intel_rows"] = rows
            else:
                _skip_block_body(tz)


def _parse_countries(tz, by_tag, wars, subjects):
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        rec = {
            "relations": {}, "wargoals": [],
            "agency_name": None, "operatives": [],
            "intel_rows": 0,
        }
        for key, op in iter_block_entries(tz):
            if op is LBRACE:
                if key == "diplomacy":
                    _parse_diplomacy(tz, rec, wars, subjects, tag)
                elif key == "intelligence_agency":
                    _parse_intelligence_agency(tz, rec)
                elif key == "intel":
                    _parse_intel(tz, rec)
                else:
                    _skip_block_body(tz)
            # scalars ignored
        by_tag[tag] = rec


def parse_diplo_text(text: str):
    by_tag: dict = {}
    factions: list = []
    wars: list = []
    subjects: list = []
    meta = dc.top_level_pass(
        text,
        {
            "faction": lambda tz: _parse_faction(tz, factions),
            "countries": lambda tz: _parse_countries(tz, by_tag, wars, subjects),
        },
        scalar_keys=("date", "version", "player"),
    )
    return by_tag, factions, wars, subjects, meta


def _dedup_wars(wars):
    """Collapse two-directional war_relation entries into one war per pair."""
    seen = {}
    for w in wars:
        a, b = w.get("first"), w.get("second")
        if not a or not b:
            continue
        key = tuple(sorted((a, b)))
        if key in seen:
            continue
        seen[key] = {
            "a": w["first"], "b": w["second"],
            "start_date": w["start_date"],
            "a_casualties": w["first_casualties"],
            "b_casualties": w["second_casualties"],
            "war_score_a_vs_b": w["score_first_vs_second"],
            "war_score_b_vs_a": w["score_second_vs_first"],
        }
    return list(seen.values())


def build(text, source, was_melted):
    by_tag, factions, wars, subjects, meta = parse_diplo_text(text)

    # faction back-reference per country.
    tag_to_faction = {}
    for f in factions:
        for m in f["members"]:
            tag_to_faction[m] = f["id"]

    countries = {}
    for tag, rec in by_tag.items():
        countries[tag] = {
            "faction_id": tag_to_faction.get(tag),
            "relations": rec["relations"],
            "wargoals": rec["wargoals"],
            "agency_name": rec["agency_name"],
            "operatives": rec["operatives"],
            "intel_rows": rec["intel_rows"],
        }

    deduped_wars = _dedup_wars(wars)
    ordered_war_rows = sorted(
        wars,
        key=lambda row: (
            row.get("owner") or "",
            row.get("relation_other") or "",
            row.get("first") or "",
            row.get("second") or "",
        ),
    )
    # dedup subjects by (overlord, subject)
    subj_seen = {}
    for p in subjects:
        ov, su = p.get("first"), p.get("second")
        if not ov or not su:
            continue
        subj_seen[(ov, su)] = {
            "overlord": ov, "subject": su,
            "autonomy_state": p.get("autonomy_state"),
            "value": p.get("value"), "start_date": p.get("start_date"),
        }
    subjects_out = list(subj_seen.values())

    total_ops = sum(len(c["operatives"]) for c in countries.values())
    out = {
        "meta": {
            "source_filename": source.name,
            "source_path_redacted": dc.ps._redact(source),
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "notes": {
                "wars": "no top-level war section; derived from pairwise "
                "war_relation entries (collapsed). war_score_* = province_capture "
                "component (dominant term).",
                "intel_levels": "intel.intel is a POSITIONAL array by internal "
                "country index, not tag; only row count is reported here.",
            },
            "stats": {
                "factions": len(factions),
                "wars": len(deduped_wars),
                "war_relation_rows": len(ordered_war_rows),
                "subject_relationships": len(subjects_out),
                "countries": len(countries),
                "total_operatives": total_ops,
            },
        },
        "factions": factions,
        "wars": deduped_wars,
        "war_relation_rows": ordered_war_rows,
        "subjects": subjects_out,
        "countries": dict(sorted(countries.items())),
    }

    ger = countries.get("GER", {})
    lines = ["=== DIPLOMACY ORACLE VALIDATION ==="]
    lines.append(f"source        : {source.name} (melted={was_melted})")
    lines.append(f"save date     : {meta.get('date')}")
    lines.append(f"factions      : {len(factions)} "
                 f"-> {[(f['template'], len(f['members'])) for f in factions]}")
    lines.append(f"wars          : {len(deduped_wars)}")
    lines.append(f"subjects      : {len(subjects_out)} "
                 f"-> {[(s['overlord'], s['subject'], s['autonomy_state']) for s in subjects_out][:4]}")
    lines.append(f"GER relations : {len(ger.get('relations', {}))} "
                 f"agency={ger.get('agency_name')!r} "
                 f"operatives={len(ger.get('operatives', []))}")
    lines.append(f"GER wargoals  : {len(ger.get('wargoals', []))}")
    # show a known war involving GER
    ger_wars = [w for w in deduped_wars if "GER" in (w["a"], w["b"])]
    lines.append(f"GER wars      : {len(ger_wars)}")
    for w in ger_wars[:4]:
        lines.append(f"   {w['a']} vs {w['b']} since {w['start_date']} "
                     f"(cas {w['a_casualties']}/{w['b_casualties']})")
    return {"diplomacy.json": out}, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(dc.run_cli(
        "diplo", "the game save -> diplomacy & intel oracle.",
        build, ["diplomacy.json"]))
