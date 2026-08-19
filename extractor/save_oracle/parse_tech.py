#!/usr/bin/env python3
"""the game save -> per-country RESEARCH / TECHNOLOGY oracle (``tech.json``).

Deep-layer extension of the save oracle. Per country it extracts:
  - ``technologies``        : list of {name, level, date, research_points} for
                              every researched technology (the things the
                              country HAS).
  - ``tech_count``          : len(technologies)
  - ``in_progress``         : list of {name, points_factor} for the techs
                              currently in a research slot. SEE REALITY NOTE.
  - ``research_slots``      : countries.<TAG>.research_slot (current slot count)
  - ``original_research_slots`` : the starting slot count
  - ``research_bonuses``    : list of {name, bonus, ahead_reduction, claim,
                              cost_reduction, uses, categories[...]} merged from
                              ``limited_use_bonus`` and
                              ``limited_use_cost_reduction_bonus``.

Key paths (verified against a real 1.19 melt):
  countries.<TAG>.technology.technologies.<tech_name> = {
      level=1  date=1936.1.1.12  research_points=165  ... }
  countries.<TAG>.technology.slots.<tech_name> = { points_factor=1.31 }
      -> the ACTIVE research queue (one entry per occupied slot)
  countries.<TAG>.technology.limited_use_bonus = {
      bonus=1  ahead_reduction=2  claim=1  uses=1  id=..  name="..."
      category=synth_resources  category=... }     # category may repeat
  countries.<TAG>.technology.limited_use_cost_reduction_bonus = {
      cost_reduction=0.5  uses=2  id=..  name="..." }
  countries.<TAG>.research_slot           = 5
  countries.<TAG>.original_research_slots = 4

REALITY NOTE -- research progress: ``technology.slots`` stores the active tech
name and ``points_factor`` speed. In current fixtures the same active tech also
appears in ``technology.technologies`` with ``level = null``, ``date =
"1.1.1.1"``, and ``research_points`` = accumulated active progress points. This
parser preserves both raw facts; ``models.research_progress`` performs the
derived fraction/ETA join against static tech costs and fails closed when any
input is absent.

stdlib only. Reuses the shared tokenizer via ``deep_common``.
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    LBRACE, atom_value, iter_block_entries, _skip_block_body, to_int, to_float,
)


def _parse_technologies(tz):
    """technologies={ <tech>={ level date research_points ... } ... }."""
    out = []
    for tech_name, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        level = date = rp = None
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                _skip_block_body(tz)
                continue
            v = atom_value(op)
            if v is None:
                continue
            if k == "level":
                level = to_int(v)
            elif k == "date":
                date = v
            elif k == "research_points":
                rp = to_float(v)
        out.append({"name": tech_name, "level": level,
                    "date": date, "research_points": rp})
    return out


def _parse_slots(tz):
    """slots={ <tech>={ points_factor=.. } ... } -> active research queue."""
    out = []
    for tech_name, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        pf = None
        for k, op in iter_block_entries(tz):
            if op is LBRACE:
                _skip_block_body(tz)
                continue
            v = atom_value(op)
            if v is None:
                continue
            if k == "points_factor":
                pf = to_float(v)
        out.append({"name": tech_name, "points_factor": pf})
    return out


def _parse_bonus(tz, cost_reduction_kind: bool):
    """Read a limited_use_bonus / limited_use_cost_reduction_bonus block."""
    rec = {"name": None, "categories": []}
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "name":
            rec["name"] = v
        elif k == "category":
            rec["categories"].append(v)
        elif k in ("bonus", "ahead_reduction", "claim", "cost_reduction"):
            rec[k] = to_float(v)
        elif k == "uses":
            rec["uses"] = to_int(v)
    rec["kind"] = "cost_reduction" if cost_reduction_kind else "research_bonus"
    return rec


def _parse_technology_block(tz, rec):
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "technologies":
                rec["technologies"] = _parse_technologies(tz)
            elif key == "slots":
                rec["in_progress"] = _parse_slots(tz)
            elif key == "limited_use_bonus":
                rec["research_bonuses"].append(_parse_bonus(tz, False))
            elif key == "limited_use_cost_reduction_bonus":
                rec["research_bonuses"].append(_parse_bonus(tz, True))
            else:
                _skip_block_body(tz)


def _parse_countries(tz, by_tag):
    for tag, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        rec = {
            "technologies": [],
            "tech_count": 0,
            "in_progress": [],
            "research_slots": None,
            "original_research_slots": None,
            "research_bonuses": [],
        }
        for key, op in iter_block_entries(tz):
            if op is LBRACE:
                if key == "technology":
                    _parse_technology_block(tz, rec)
                else:
                    _skip_block_body(tz)
                continue
            v = atom_value(op)
            if v is None:
                continue
            if key == "research_slot":
                rec["research_slots"] = to_int(v)
            elif key == "original_research_slots":
                rec["original_research_slots"] = to_int(v)
        rec["tech_count"] = len(rec["technologies"])
        if rec["technologies"] or rec["research_slots"] is not None:
            by_tag[tag] = rec


def parse_tech_text(text: str):
    by_tag: dict = {}
    meta = dc.top_level_pass(
        text,
        {"countries": lambda tz: _parse_countries(tz, by_tag)},
        scalar_keys=("date", "version", "player"),
    )
    return by_tag, meta


def build(text, source, was_melted):
    by_tag, meta = parse_tech_text(text)

    total_tech = sum(r["tech_count"] for r in by_tag.values())
    researching = sum(1 for r in by_tag.values() if r["in_progress"])
    out = {
        "meta": {
            "source_filename": source.name,
            "source_path_redacted": dc.ps._redact(source),
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "notes": {
                "research_progress": "active slot points_factor is saved; "
                "matching technologies[] rows with date 1.1.1.1 can carry "
                "accumulated research_points. Derived progress is computed "
                "outside the raw parser.",
            },
            "stats": {
                "countries_with_tech": len(by_tag),
                "total_technologies": total_tech,
                "countries_actively_researching": researching,
            },
        },
        "countries": dict(sorted(by_tag.items())),
    }

    ger = by_tag.get("GER", {})
    lines = ["=== TECH ORACLE VALIDATION ==="]
    lines.append(f"source        : {source.name} (melted={was_melted})")
    lines.append(f"save date     : {meta.get('date')}")
    lines.append(f"countries w/ tech  : {len(by_tag)}")
    lines.append(f"total techs        : {total_tech}")
    lines.append(f"actively researching: {researching} countries")
    lines.append(f"GER techs          : {ger.get('tech_count')} "
                 f"(slots={ger.get('research_slots')}/"
                 f"orig {ger.get('original_research_slots')})")
    lines.append(f"GER in-progress    : {len(ger.get('in_progress', []))} "
                 f"-> {[s['name'] for s in ger.get('in_progress', [])][:4]}")
    lines.append(f"GER research bonuses: {len(ger.get('research_bonuses', []))}")
    if ger.get("technologies"):
        t0 = ger["technologies"][0]
        lines.append(f"GER sample tech    : {t0['name']} "
                     f"lvl={t0['level']} date={t0['date']}")
    return {"tech.json": out}, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(dc.run_cli(
        "tech", "the game save -> per-country research/technology oracle.",
        build, ["tech.json"]))
