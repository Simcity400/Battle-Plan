#!/usr/bin/env python3
"""the game save -> ACTIVE COMBATS oracle (``combats.json``).

Deep-layer extension of the save oracle. Extracts the active battle structures
that the save exposes at the top-level ``combat={}`` section. This seeds the
later high-frequency combat work; it documents exactly what the SAVE has vs what
will only ever come from live memory.

Per combat (top-level ``combat.land_combat`` — see REALITY NOTE on naval/air):
  - ``combat_id``  : the combat (id,type).
  - ``kind``       : "land" / "naval" / "air".
  - ``location``   : province id where the battle is happening.
  - ``day`` / ``duration`` : engine day stamp + how long the battle has run.
  - ``terrain``
  - ``attacker`` / ``defender`` : {
        units      : list of participating unit (division/ship) ids,
        unit_count : len(units),
        losses, air_damage_str, air_damage_org,
        ground_damage_str, ground_damage_org, tactic,
        progress   : the side's battle-progress value (from its log block) }

Key paths (verified against a real 1.19 melt):
  combat.land_combat = {
      id={id type} location=<prov> day=44 duration=44 terrain="hills"
      attacker={ unit={id type}... losses size air_damage_str air_damage_org
                 ground_damage_str ground_damage_org tactic
                 log={ ... progress=14.55 } }
      defender={ ... log={ ... progress=135.41 } } }

REALITY NOTE — what the save does vs does NOT expose:
  - This melt contains ONLY ``land_combat`` entries; there are NO
    ``naval_combat`` / ``air_combat`` blocks in the save. Naval and air battles
    are transient and are NOT persisted to the melted save — they live in live
    memory only. We still scan for ``naval_combat``/``air_combat`` keys so the
    schema is future-proof, and report what was/wasn't found.
  - The combat ``progress`` is per SIDE (attacker.log.progress vs
    defender.log.progress). It is the engine's accumulating combat-resolution
    figure, not a 0-100 percentage; reported verbatim.
  - Unit lists are bare (id,type) handle refs — they join to the
    division order-of-battle in divisions.json by id, NOT resolved here.

stdlib only. Reuses the shared tokenizer via ``deep_common``.
"""

from __future__ import annotations

import deep_common as dc
from deep_common import (
    LBRACE, atom_value, iter_block_entries, _skip_block_body,
    to_int, to_float, read_id_block,
)


def _parse_log_progress(tz):
    """Read a combat side's log={...} block and return its 'progress' value."""
    progress = None
    for k, op in iter_block_entries(tz):
        if op is LBRACE:
            _skip_block_body(tz)
            continue
        v = atom_value(op)
        if v is None:
            continue
        if k == "progress":
            progress = to_float(v)
    return progress


def _parse_side(tz):
    rec = {
        "units": [], "unit_count": 0,
        "losses": None, "air_damage_str": None, "air_damage_org": None,
        "ground_damage_str": None, "ground_damage_org": None,
        "tactic": None, "progress": None,
    }
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "unit":
                _id, _t = read_id_block(tz)
                if _id is not None:
                    rec["units"].append([_id, _t])
            elif key == "log":
                rec["progress"] = _parse_log_progress(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "losses":
            rec["losses"] = to_float(v)
        elif key == "air_damage_str":
            rec["air_damage_str"] = to_float(v)
        elif key == "air_damage_org":
            rec["air_damage_org"] = to_float(v)
        elif key == "ground_damage_str":
            rec["ground_damage_str"] = to_float(v)
        elif key == "ground_damage_org":
            rec["ground_damage_org"] = to_float(v)
        elif key == "tactic":
            rec["tactic"] = to_int(v)
    rec["unit_count"] = len(rec["units"])
    return rec


def _parse_one_combat(tz, kind):
    rec = {
        "combat_id": None, "kind": kind,
        "location": None, "day": None, "duration": None, "terrain": None,
        "attacker": None, "defender": None,
    }
    for key, opener in iter_block_entries(tz):
        if opener is LBRACE:
            if key == "id":
                _id, _t = read_id_block(tz)
                rec["combat_id"] = [_id, _t]
            elif key == "attacker":
                rec["attacker"] = _parse_side(tz)
            elif key == "defender":
                rec["defender"] = _parse_side(tz)
            else:
                _skip_block_body(tz)
            continue
        v = atom_value(opener)
        if v is None:
            continue
        if key == "location":
            rec["location"] = to_int(v)
        elif key == "day":
            rec["day"] = to_int(v)
        elif key == "duration":
            rec["duration"] = to_int(v)
        elif key == "terrain":
            rec["terrain"] = v
    return rec


def _parse_combat_section(tz, combats, kind_counts):
    """combat={ land_combat={..} naval_combat={..} air_combat={..} ... }."""
    for key, opener in iter_block_entries(tz):
        if opener is not LBRACE:
            continue
        if key in ("land_combat", "naval_combat", "air_combat"):
            kind = key.replace("_combat", "")
            kind_counts[kind] = kind_counts.get(kind, 0) + 1
            combats.append(_parse_one_combat(tz, kind))
        else:
            # 'history' (ended-combat archive) and anything else -> skip
            _skip_block_body(tz)


def parse_combats_text(text: str):
    combats: list = []
    kind_counts: dict = {}
    meta = dc.top_level_pass(
        text,
        {"combat": lambda tz: _parse_combat_section(tz, combats, kind_counts)},
        scalar_keys=("date", "version", "player"),
    )
    return combats, kind_counts, meta


def build(text, source, was_melted):
    combats, kind_counts, meta = parse_combats_text(text)

    out = {
        "meta": {
            "source_filename": source.name,
            "source_path_redacted": dc.ps._redact(source),
            "was_melted": was_melted,
            "save_date": meta.get("date"),
            "save_version": meta.get("version"),
            "player": meta.get("player"),
            "notes": {
                "naval_air": "this save exposes only land_combat; naval/air "
                "battles are transient and NOT persisted to the melt (live-memory "
                "only). Keys are scanned for anyway.",
                "progress": "per-side log.progress is the engine's accumulating "
                "combat figure, not a 0-100 percent.",
                "units": "side unit ids are (id,type) handle refs; join to "
                "divisions.json by id for the order of battle.",
            },
            "stats": {
                "total_combats": len(combats),
                "by_kind": kind_counts,
            },
        },
        "combats": combats,
    }

    lines = ["=== COMBAT ORACLE VALIDATION ==="]
    lines.append(f"source        : {source.name} (melted={was_melted})")
    lines.append(f"save date     : {meta.get('date')}")
    lines.append(f"combats       : {len(combats)}  by kind={kind_counts}")
    lines.append("(naval/air combats are NOT persisted to saves; land only)")
    for c in combats[:4]:
        a = c["attacker"] or {}
        d = c["defender"] or {}
        lines.append(
            f"   combat {c['combat_id']} @prov {c['location']} "
            f"terrain={c['terrain']} day={c['day']} dur={c['duration']}: "
            f"atk {a.get('unit_count')} units prog={a.get('progress')} "
            f"vs def {d.get('unit_count')} units prog={d.get('progress')}")
    return {"combats.json": out}, "\n".join(lines)


if __name__ == "__main__":
    raise SystemExit(dc.run_cli(
        "combats", "the game save -> active combats oracle.",
        build, ["combats.json"]))
